//! The compositor state: the Wayland protocol handlers, the desktop and
//! the core. It is the host of the core on Wayland, as `Runtime` is on
//! Windows, and it does not depend on how frames reach a screen (nested
//! window, DRM, or nothing at all in tests).

use gevrek_host::wm::bindings::Bindings as _;
use gevrek_host::wm::reload::Reload as _;
use crate::desktop::{WlDesktop, WlOutput};
use gevrek_engine::core::WmCore;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::executor::Executor;
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::outputs::{MonitorHandle, MonitorInfo, Outputs};
use gevrek_engine::{Desktop, Rect, WindowHandle};
use gevrek_model::command::DispatchCommand;
use gevrek_model::config::Config;
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use crate::surface::{KeyTarget, StateFlag, WindowSurface};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::utils::{Serial, Transform, SERIAL_COUNTER};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
    CompositorState,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::desktop::{PopupKind, PopupManager};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::wayland::shm::{ShmHandler, ShmState};
use std::sync::{Arc, Mutex};

pub struct State {
    pub dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    pub dmabuf_global: Option<smithay::wayland::dmabuf::DmabufGlobal>,
    /// The graphics card programs are told to draw with: their buffers
    /// are marked as made on it.
    pub dmabuf_node: Option<smithay::backend::drm::DrmNode>,
    /// The backend's test of a client's dmabuf (see `dmabuf::offer`).
    pub dmabuf_import: Option<crate::dmabuf::ImportTest>,
    /// A client has handed over a dmabuf (said once in the log).
    pub dmabuf_used: bool,
    /// Where pictures of the screen may be drawn straight into a
    /// program's own buffer of the graphics card: the card (as the
    /// system numbers its device) and the kinds of buffer it draws
    /// into, each with the layouts it takes. `None`: only through
    /// shared memory.
    pub(crate) capture_card: Option<crate::dmabuf::CaptureCard>,
    /// The output each window's surfaces were last told they are on, and
    /// its scale.
    pub surface_outputs: std::collections::HashMap<WindowHandle, (MonitorHandle, f64)>,
    pub display: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub decoration_state: XdgDecorationState,
    /// xdg-dialog: a program says of a window that it is a dialog to be
    /// answered first (modal); such a window floats.
    pub dialog_state: smithay::wayland::shell::xdg::dialog::XdgDialogState,
    /// Programs are told when a frame of theirs reached the screen
    /// (presentation-time), by the clock that does not jump.
    pub presentation_state: smithay::wayland::presentation::PresentationState,
    /// Explicit sync (linux-drm-syncobj): a program says with each
    /// picture when the card will have drawn it and is told when gevrek is
    /// done with it. Only on hardware whose driver can do it.
    pub syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    /// Other programs' display tools (`output_management.rs`).
    pub(crate) output_management: crate::output_management::OutputManagement,
    /// Other panels' taskbars (`toplevel_management.rs`).
    pub(crate) toplevel_management: crate::toplevel_management::ToplevelManagement,
    /// Other panels' workspace lists (`workspace_management.rs`).
    pub(crate) workspace_management: crate::workspace_management::WorkspaceManagement,
    /// Frames of the newer capture protocol that wait for their picture.
    pub(crate) image_captures: Vec<crate::image_capture::PendingImage>,
    /// The programs that follow the pointer by itself (its place over a
    /// display or a window, and its picture).
    pub(crate) pointer_watches: Vec<crate::image_capture::PointerWatch>,
    /// What the pointer's picture is drawn into for them.
    pub(crate) pointer_target: Option<Box<dyn std::any::Any>>,
    /// The program that is the input method (the first that asked, for
    /// as long as it is there): a second one is told it is not taken.
    pub(crate) input_method_of: Option<smithay::reexports::wayland_server::backend::ClientId>,
    /// What a Wayland program drags now (its source of data), for the
    /// case that it comes over an X11 window; and gevrek as that drag's
    /// source for X11 windows, once it has (`xdnd.rs`).
    pub(crate) dragged: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
    pub(crate) xdnd_out: Option<crate::xdnd::Out>,
    /// The other way round: an X11 program's drag (`xdnd.rs`).
    pub(crate) x_drag: crate::xwayland::XDrag,
    /// A Wayland program's drag is on, and the action the keys held
    /// asked of it when they were last looked at (`dnd.rs`).
    pub(crate) dnd_on: bool,
    pub(crate) drag_asked:
        Option<smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction>,
    /// Which displays other programs take pictures of again and again:
    /// the screen is shared or recorded (`sharing.rs`).
    pub(crate) sharing: crate::sharing::Sharing<gevrek_engine::outputs::MonitorHandle>,
    /// How often each output was drawn anew (and all of them at once),
    /// for the capture sessions that wait for a change.
    pub(crate) capture_changes: std::collections::HashMap<gevrek_engine::outputs::MonitorHandle, u64>,
    pub(crate) capture_changes_everywhere: u64,
    /// Counts the icons programs gave their windows (each file of one
    /// has another name).
    pub(crate) icon_serial: u32,
    pub pointer_gestures_state: smithay::wayland::pointer_gestures::PointerGesturesState,
    pub alpha_modifier_state: smithay::wayland::alpha_modifier::AlphaModifierState,
    pub system_bell_state: smithay::wayland::xdg_system_bell::XdgSystemBellState,
    pub toplevel_tag_state: smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager,
    pub output_manager_state: smithay::wayland::output::OutputManagerState,
    pub viewporter_state: smithay::wayland::viewporter::ViewporterState,
    pub fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub cursor_shape_state: smithay::wayland::cursor_shape::CursorShapeManagerState,
    pub primary_selection_state:
        smithay::wayland::selection::primary_selection::PrimarySelectionState,
    pub idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    // more_protocols.rs
    pub relative_pointer_state: smithay::wayland::relative_pointer::RelativePointerManagerState,
    pub pointer_constraints_state: smithay::wayland::pointer_constraints::PointerConstraintsState,
    #[allow(dead_code)]
    pub(crate) x11_key_grab_state:
        smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState,
    #[allow(dead_code)]
    pub(crate) fifo_state: smithay::wayland::fifo::FifoManagerState,
    #[allow(dead_code)]
    pub(crate) commit_timing_state: smithay::wayland::commit_timing::CommitTimingManagerState,
    /// X11 windows that said "fullscreen" before they were shown, by
    /// their X11 number, until they are shown.
    pub(crate) x11_start_fullscreen: Vec<u32>,
    /// The X11 windows that asked never to be handed to the display
    /// past the compositor (`_NET_WM_BYPASS_COMPOSITOR` 2).
    pub(crate) x11_composited: std::collections::HashSet<u32>,
    /// The display the next new window is to open on, and since when
    /// that is asked (`DispatchCommand::ExpectWindow`).
    pub(crate) expected_window: Option<(String, std::time::Instant)>,
    /// The keyboard layout in use, as it was last looked at.
    pub(crate) keyboard_layout: Option<gevrek_engine::overlay_data::KeyboardLayoutNow>,
    /// The grab an X11 program holds the keyboard with, while it does.
    pub(crate) x11_key_grab: Option<
        smithay::reexports::wayland_protocols::xwayland::keyboard_grab::zv1::server::zwp_xwayland_keyboard_grab_v1::ZwpXwaylandKeyboardGrabV1,
    >,
    pub shortcuts_inhibit_state:
        smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
    pub xdg_foreign_state: smithay::wayland::xdg_foreign::XdgForeignState,
    pub wlr_data_control_state: smithay::wayland::selection::wlr_data_control::DataControlState,
    pub ext_data_control_state: smithay::wayland::selection::ext_data_control::DataControlState,
    pub single_pixel_buffer_state: smithay::wayland::single_pixel_buffer::SinglePixelBufferState,
    pub content_type_state: smithay::wayland::content_type::ContentTypeState,
    pub virtual_keyboard_state: smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState,
    // input_method.rs
    pub text_input_state: smithay::wayland::text_input::TextInputManagerState,
    pub input_method_state: smithay::wayland::input_method::InputMethodManagerState,
    // lock.rs
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// The session is locked: only lock surfaces are drawn and reachable.
    pub locked: bool,
    pub(crate) lock_surfaces: Vec<(MonitorHandle, smithay::wayland::session_lock::LockSurface)>,
    /// The lock of the program that locked the session: no other
    /// locks or unlocks it while that one lives.
    pub(crate) locker: Option<
        smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1,
    >,
    /// Made when the event loop is known (`session::open`).
    pub(crate) idle_notifier: Option<smithay::wayland::idle_notify::IdleNotifierState<Self>>,
    /// The pointer's place with its fraction of a pixel: slow movement
    /// adds up instead of being rounded away.
    pub(crate) pointer_exact: (f64, f64),
    /// Surfaces that ask the session not to go idle (a playing video).
    pub idle_inhibitors: std::collections::HashSet<WlSurface>,
    /// The displays are switched off (by the key for it, or after a time
    /// without input), since when. A key or the mouse switches them on.
    pub displays_off: Option<std::time::Instant>,
    /// When the displays were last switched off for want of input (so
    /// that it happens once for each time the user goes away).
    pub(crate) idled_off: bool,
    /// When a key or the mouse was last used.
    pub(crate) last_activity: std::time::Instant,
    pub foreign_toplevels: smithay::wayland::foreign_toplevel_list::ForeignToplevelListState,
    /// The entry of each window in the window list external bars read.
    pub foreign_handles: std::collections::HashMap<
        WindowHandle,
        smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle,
    >,
    pub activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    /// The X11 window manager side, once Xwayland is ready.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    pub x_display: Option<u32>,
    /// Xwayland as a client, and the scale it was last told X11 programs
    /// draw at (`tell_x11_scale`).
    pub(crate) x_client: Option<smithay::reexports::wayland_server::Client>,
    pub(crate) x_scale: f64,
    /// The event loop, for what has to add a source later (X11 clipboard
    /// transfers).
    pub loop_handle: Option<smithay::reexports::calloop::LoopHandle<'static, State>>,
    /// X11 windows that place themselves (menus, tooltips), bottom to top.
    pub x11_overrides: Vec<smithay::xwayland::X11Surface>,
    /// A layer surface (a launcher, a bar with a text field) that has the
    /// keyboard instead of a window.
    pub layer_focus: Option<WlSurface>,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
    /// Cleared to leave the event loop.
    pub running: bool,

    pub config: Config,
    pub desktop: WlDesktop,
    pub core: WmCore,
    /// The Wayland outputs, by the id the engine knows them under.
    pub outputs: Vec<(MonitorHandle, Output)>,
    pub start_time: std::time::Instant,
    /// The window rules, shared with the desktop and the executor.
    pub rules: Arc<Mutex<gevrek_engine::rules::RuleEngine>>,
    /// What the host owns, the same on every system (`gevrek_host::wm`):
    /// the link to the IPC server (when there is one), the startup
    /// programs, the live binding submap, the event stream.
    pub host: gevrek_host::wm::HostState,
    /// The bindings that are live now, by registration id.
    pub bindings: Vec<(i32, gevrek_engine::input::Hotkey)>,
    /// Execution is paused: bindings and startup programs rest.
    pub paused: bool,
    /// Keys whose press a binding kept from the client: their release is
    /// kept too.
    pub suppressed_keys: std::collections::HashSet<u32>,
    /// Each key that is held, as it went down: the modifiers that were
    /// down then (Ctrl, Alt, Shift, Super), and the bindings on its
    /// release that were live then. Its release runs those, whatever is
    /// still held when it comes up and whatever set of bindings is live
    /// by then. Whoever holds Super+Z and lets go of Super first has
    /// still let go of Super+Z; and a press that goes into another set
    /// of bindings is let go in the set it was pressed in.
    pub(crate) pressed_with: crate::input::Strokes,
    /// The same for the mouse's buttons.
    pub(crate) buttons_pressed_with: crate::input::Strokes,
    /// Bindings a key hit that let the key through to the client.
    pub pending_bindings: Vec<(
        Vec<i32>,
        String,
        smithay::input::keyboard::ModifiersState,
        gevrek_model::settings::BindingTrigger,
    )>,
    /// What the pointer looks like, as the client under it asked.
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    /// No host session draws the pointer (real hardware): gevrek draws an
    /// arrow for any named cursor.
    pub draw_own_cursor: bool,
    /// The built-in arrow, as a picture for the renderer.
    pub arrow: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    /// The user asked for another virtual terminal (Ctrl+Alt+F<n>); the
    /// hardware backend carries it out.
    pub vt_request: Option<i32>,
    /// gevrek owns a virtual terminal (the hardware backend).
    pub vt_switching: bool,
    /// A touchpad swipe in progress, and what the state snapshot says of
    /// the last one.
    pub swipe: Option<crate::gestures::Swipe>,
    /// The outputs exist only in memory (`headless.rs`), and how many of
    /// them were made so far (for their names).
    pub headless: bool,
    pub(crate) virtual_outputs: usize,
    /// The backend changes modes and switches displays on and off while it
    /// runs (the hardware, and the outputs in memory); such changes are
    /// then applied on trial.
    pub live_outputs: bool,
    /// The displays whose refresh rate follows the program that fills
    /// them (`variable_refresh`, where the display and its driver can).
    pub vrr_on: std::collections::HashSet<String>,
    /// The night light is asked for and no display here can show it (a
    /// window in another session, outputs in memory, a driver without a
    /// colour ramp): told as a notice.
    pub night_light_impossible: bool,
    /// A colour temperature that is only shown to be looked at, and until
    /// when (`DispatchCommand::NightLightPreview`).
    pub night_preview: Option<(u32, std::time::Instant)>,
    /// How the frames of each display got there (the hardware backend
    /// counts): a window's own picture handed over, frames drawn here,
    /// and which the last one was.
    pub frames_shown: std::collections::HashMap<MonitorHandle, (u64, u64, bool)>,
    /// How dark the picture is as a warning that the screen is about to
    /// lock by itself: twentieths, 0 (as it is) to 20.
    /// The change between desktop and lock screen that is being drawn.
    pub(crate) lock_change: Option<crate::lock::LockChange>,
    /// This gevrek has left the display: all of it is under its veil.
    pub(crate) veiled: bool,
    /// The dark at the edges of that change (small; drawn large).
    pub(crate) vignette: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    pub(crate) lock_warning: u8,
    pub(crate) lock_warning_id: smithay::backend::renderer::element::Id,
    pub(crate) lock_warning_commit: smithay::backend::renderer::utils::CommitCounter,
    /// `desktop.outputs` changed: the backend has modes to bring in line.
    pub(crate) output_rescan: bool,
    /// New display settings wait to be kept (`output_trial.rs`).
    pub(crate) output_trial: Option<crate::output_trial::OutputTrial>,
    /// The sizes the outputs in memory were made with, in device pixels.
    pub(crate) virtual_sizes: Vec<(MonitorHandle, (i32, i32))>,
    /// The direction the turning gradient was last drawn at.
    pub(crate) drawn_gradient_angle: f32,
    /// The wallpaper's pictures, one for each display (`wallpaper.rs`).
    pub(crate) wallpapers: std::cell::RefCell<crate::wallpaper::Wallpapers>,
    /// The shadows under the surfaces of shells whose rule asks for one
    /// (`desktop.layer_rules`), each drawn once for what it is under.
    pub(crate) layer_shadows: std::cell::RefCell<
        Vec<(
            smithay::reexports::wayland_server::Weak<
                smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
            >,
            gevrek_engine::decoration_render::DecorationSpec,
            smithay::backend::renderer::element::memory::MemoryRenderBuffer,
        )>,
    >,
    /// The name the solid colour behind everything is drawn under.
    pub(crate) backdrop_id: smithay::backend::renderer::element::Id,
    /// Counted up when the colour changes, so that it is drawn anew.
    pub(crate) backdrop_commit: smithay::backend::renderer::utils::CommitCounter,
    /// Frames other programs asked for (`capture.rs`), waiting for the
    /// next drawing of their output.
    pub(crate) captures: Vec<crate::capture::PendingCapture>,
    /// The elements are being made for a picture without the pointer.
    pub(crate) hide_pointer: bool,
    /// The elements are being made for a picture another program takes
    /// of the screen: without the surfaces whose rule keeps them out of
    /// such pictures (`sharing.rs`).
    pub(crate) capture_pass: bool,
    /// The elements are being made for the picture of what lies behind
    /// the see-through surfaces: without those surfaces (`soft.rs`).
    pub(crate) backdrop_pass: bool,
    /// The soft pictures of what lies behind them.
    pub(crate) soft: std::cell::RefCell<crate::soft::Soft>,
    /// A window is being picked by a press on it (`pick.rs`): what the
    /// pointer looked like before, to be put back.
    pub(crate) picking: Option<smithay::input::pointer::CursorImageStatus>,
    /// The mouse button whose press picked: its release is nobody's.
    pub(crate) pick_button: Option<u32>,
    /// Escape was pressed while picking: it ends after the key.
    pub(crate) pick_cancel_asked: bool,
    /// The Escape key that ended it, until that key is let go.
    pub(crate) pick_escape: Option<u32>,
    /// X11 programs are to run (Xwayland was started), and the program
    /// named on the command line that waits for it.
    pub xwayland_wanted: bool,
    /// When Xwayland was started again after it ended, lately: it is not
    /// started again and again when it does not stay up.
    pub(crate) xwayland_restarts: Vec<std::time::Instant>,
    /// When the programs were last told that a frame was shown, and
    /// whether they are owed that word once enough time has passed (the
    /// hardware backend tells once for each frame of the display).
    pub(crate) frames_told: std::time::Instant,
    pub(crate) frames_owed: bool,
    /// A timer is set that wakes the loop when a picture a program holds
    /// back for a time of its own is due (the hardware backend).
    pub(crate) paced_wake: bool,
    /// The overview of every workspace, while it is up (`overview.rs`).
    pub workspace_overview: Option<crate::overview::WorkspaceOverview>,
    /// What stands around the pictures of the overview of one workspace's
    /// windows, and what chooses among them (`window_overview.rs`).
    pub window_overview: crate::window_overview::WindowOverview,
    /// Escape was pressed while it was up: it is put away after the key.
    pub(crate) overview_leave_asked: bool,
    /// The Escape key that put it away, until that key is let go.
    pub(crate) overview_escape: Option<u32>,
    /// The mouse button whose press chose something in it: its release
    /// is nobody's.
    pub(crate) overview_button: Option<u32>,
    /// The swipe that is going on pulls the workspace beside into view
    /// (`gestures.rs`).
    pub swipe_pull: Option<crate::gestures::Pull>,
    /// The swipe that is going on is passed on to the program under the
    /// pointer (it is none of the configuration's gestures).
    pub swipe_forwarded: bool,
    pub pending_program: Option<gevrek_model::settings::LaunchProfile>,
    /// gevrek ends when that program does.
    pub ends_with_program: bool,
    /// How often the host passes, and how it is woken (`session.rs`).
    pub host_pace: crate::session::HostPace,
    /// The input devices libinput reported (hardware only), and the keymap
    /// that was last set (`devices.rs`).
    pub(crate) input_devices: Vec<smithay::reexports::input::Device>,
    pub(crate) applied_keymap: Option<(String, String, String, String)>,
    /// The picture of what is being dragged, if a drag is on and has one;
    /// and how far its program has moved it from the pointer (the place
    /// of the picture that is under the pointer, taken the other way).
    pub(crate) drag_icon: Option<WlSurface>,
    pub(crate) drag_icon_offset: (i32, i32),
    /// gevrek is the session the user logged into (`--session`): the
    /// service manager and D-Bus are told its environment.
    pub session_mode: bool,
    /// When Ctrl+Alt+Backspace was last pressed (twice in a row ends the
    /// session, whatever the configuration says).
    pub(crate) rescue_pressed: Option<std::time::Instant>,
    /// The cursor theme (`cursor.rs`).
    pub(crate) cursors: crate::cursor::Cursors,
    /// The clock of made-up input (`inject.rs`).
    pub(crate) injected_time: u32,
    pub gesture_progress: Option<gevrek_engine::overlay_data::GestureProgressData>,
    /// The input backend delivers touchpad swipes (libinput does; the
    /// nested window does not).
    pub gestures_available: bool,
    /// The globals of the outputs, to withdraw them when one goes.
    pub output_globals: std::collections::HashMap<
        MonitorHandle,
        smithay::reexports::wayland_server::backend::GlobalId,
    >,
    /// `start` ran: outputs that appear from now on are set up at once.
    pub started: bool,
    /// A window being moved or resized with the pointer.
    pub drag: Option<Drag>,
    /// Keys and buttons held down that a repeat or long-press binding
    /// waits on.
    pub held: Vec<Held>,
    /// The corner of a display the pointer is in, and since when
    /// (`desktop.hot_corners`).
    pub hot_corners: gevrek_engine::hot_corners::Watch,
    /// Keys that went down in the key event being handled.
    pub pending_holds: Vec<(u32, String)>,
    /// Mouse buttons whose press a binding kept from the client.
    pub suppressed_buttons: std::collections::HashSet<u32>,
    /// The menu that is open with a grab, if one is (`XdgShellHandler::grab`).
    pub(crate) popup_grab: Option<smithay::desktop::PopupGrab<Self>>,
    /// Write the next frame to this file (a PPM picture), for looking at
    /// what the WM draws and for the checks.
    pub screenshot: Option<std::path::PathBuf>,
    /// How many windows were given a starting place.
    cascade: usize,
    next_output: isize,
}

/// A key or button that is down and that a repeat or long-press binding
/// waits on.
#[derive(Debug, Clone)]
pub struct Held {
    /// The key code, or the button code for a mouse button.
    pub code: u32,
    pub device: gevrek_model::settings::BindingDevice,
    pub name: String,
    /// Ctrl, Alt, Shift, Super when it went down.
    pub modifiers: (bool, bool, bool, bool),
    pub since: std::time::Instant,
    pub last_repeat: Option<std::time::Instant>,
    /// Long-press bindings (by registration id) that already ran.
    pub long_fired: Vec<i32>,
}

/// A move or resize of a window with the pointer, from where it began.
#[derive(Debug, Clone)]
pub struct Drag {
    pub id: WindowHandle,
    pub resize: bool,
    /// When resizing: the left edge moves (else the right), the top edge
    /// moves (else the bottom).
    pub from_left: bool,
    pub from_top: bool,
    /// The button that holds it.
    pub button: u32,
    pub start_pointer: (i32, i32),
    pub start_rect: Rect,
}

/// Called with every line the automation graph logs.
pub type LogCallback = Arc<dyn Fn(String, gevrek_model::graph::LogLevel, i64) + Send + Sync>;
/// Called when a node runs and for runtime events.
pub type NodeEventCallback = Arc<dyn Fn(gevrek_engine::executor::NodeExecutionEvent) + Send + Sync>;

/// What the compositor keeps per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Who it is, for the log: `pid 1234 (gevrek-shell)`; empty where it
    /// is not known.
    pub who: String,
}

impl ClientState {
    /// For a client that connected through `stream`.
    pub fn of(stream: &std::os::unix::net::UnixStream) -> Self {
        Self { who: who_is(stream), ..Default::default() }
    }

    fn named(&self) -> &str {
        if self.who.is_empty() {
            "a client"
        } else {
            &self.who
        }
    }
}

/// The process at the other end of `stream`: its id and its name.
fn who_is(stream: &std::os::unix::net::UnixStream) -> String {
    use std::os::fd::AsRawFd;
    let mut credentials = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the buffer is a `ucred` and `size` says so.
    let asked = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if asked != 0 || credentials.pid <= 0 {
        return String::new();
    }
    let name = std::fs::read_to_string(format!("/proc/{}/comm", credentials.pid)).unwrap_or_default();
    format!("pid {} ({})", credentials.pid, name.trim())
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {
        log::debug!("{} connected", self.named());
    }

    fn disconnected(&self, _client_id: ClientId, reason: DisconnectReason) {
        match reason {
            DisconnectReason::ProtocolError(error) => {
                log::warn!("{} was disconnected for a protocol error: {error}", self.named())
            }
            // (Also what the library says where the client read nothing
            // for so long that what it was sent no longer fit in its
            // socket: the client is cut off then too.)
            DisconnectReason::ConnectionClosed => log::debug!(
                "{} is disconnected: it closed its connection, or stopped reading it",
                self.named()
            ),
        }
    }
}

/// A client as the log names it: `pid 1234 (gevrek-shell)`, or
/// Xwayland.
pub(crate) fn named_client(client: &Client) -> String {
    if client
        .get_data::<smithay::xwayland::XWaylandClientData>()
        .is_some()
    {
        return "Xwayland".to_string();
    }
    client
        .get_data::<ClientState>()
        .map_or_else(|| "a client".to_string(), |state| state.named().to_string())
}

/// How long a display is held for the next new window: an application
/// takes its seconds to show its window.
const EXPECTED_WINDOW_HOLDS: std::time::Duration = std::time::Duration::from_secs(15);

impl State {
    /// A compositor with no outputs yet. Add outputs, then call `start`.
    pub fn new(display: DisplayHandle, config: Config) -> Self {
        Self::with_callbacks(display, config, None, None)
    }

    /// As `new`, with the callbacks that carry log lines and node events to
    /// the IPC server.
    pub fn with_callbacks(
        display: DisplayHandle,
        config: Config,
        log_events: Option<LogCallback>,
        node_events: Option<NodeEventCallback>,
    ) -> Self {
        let rules = Arc::new(Mutex::new(gevrek_engine::config_apply::build_rule_engine(
            &config.desktop,
        )));
        let mut executor = Executor::new_with_events(
            config.node_graph.clone(),
            Arc::new(Mutex::new(Vec::new())),
            log_events,
            node_events.clone(),
            Arc::clone(&rules),
        );
        executor.set_scratchpad_profiles(config.desktop.scratchpads.clone());
        executor.set_workspace_profiles(
            config.desktop.workspaces.clone(),
            config.desktop.appearance.clone(),
        );
        let outputs = Outputs::new();
        let snap_zone_manager = gevrek_engine::config_apply::build_snap_zone_manager(
            &config.desktop.snap_zones,
            &outputs,
            |_| false,
        );
        let core = WmCore::new(
            executor,
            outputs,
            FocusManager::with_config(config.desktop.focus.clone()),
            snap_zone_manager,
        );

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display, "seat0");
        // The keymap comes from the environment (XKB_DEFAULT_*), as usual.
        if let Err(error) = seat.add_keyboard(Default::default(), 400, 30) {
            log::error!("No keyboard: {error:?}");
        }
        seat.add_pointer();
        // Fingers on a touch screen (`input.rs`: `touch_down` and the rest).
        seat.add_touch();

        let mut desktop = WlDesktop::new(Arc::clone(&rules));
        desktop
            .animations
            .set_profile(config.desktop.animations.clone());

        let cursors = crate::cursor::Cursors::new(
            &config.desktop.platform.wayland.cursor_theme,
            config.desktop.platform.wayland.cursor_size,
        );
        let primary_selection_state =
            smithay::wayland::selection::primary_selection::PrimarySelectionState::new::<Self>(
                &display,
            );
        crate::capture::init(&display);
        crate::output_management::init(&display);
        crate::toplevel_management::init(&display);
        crate::workspace_management::init(&display);
        crate::virtual_pointer::init(&display);
        crate::image_capture::init(&display);
        crate::toplevel_icon::init(&display);
        crate::background_effect::init(&display);
        Self {
            compositor_state: CompositorState::new_v6::<Self>(&display),
            xdg_shell_state: XdgShellState::new::<Self>(&display),
            shm_state: ShmState::new::<Self>(&display, vec![]),
            data_device_state: DataDeviceState::new::<Self>(&display),
            decoration_state: XdgDecorationState::new::<Self>(&display),
            dialog_state: smithay::wayland::shell::xdg::dialog::XdgDialogState::new::<Self>(
                &display,
            ),
            output_manager_state: smithay::wayland::output::OutputManagerState::new_with_xdg_output::<
                Self,
            >(&display),
            viewporter_state: smithay::wayland::viewporter::ViewporterState::new::<Self>(&display),
            fractional_scale_state:
                smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(
                    &display,
                ),
            cursor_shape_state: smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(
                &display,
            ),
            relative_pointer_state:
                smithay::wayland::relative_pointer::RelativePointerManagerState::new::<Self>(
                    &display,
                ),
            pointer_constraints_state:
                smithay::wayland::pointer_constraints::PointerConstraintsState::new::<Self>(
                    &display,
                ),
            x11_key_grab: None,
            keyboard_layout: None,
            expected_window: None,
            x11_start_fullscreen: Vec::new(),
            x11_composited: Default::default(),
            fifo_state: smithay::wayland::fifo::FifoManagerState::new::<Self>(&display),
            commit_timing_state: smithay::wayland::commit_timing::CommitTimingManagerState::new::<
                Self,
            >(&display),
            x11_key_grab_state:
                smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<Self>(
                    &display,
                ),
            shortcuts_inhibit_state:
                smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState::new::<
                    Self,
                >(&display),
            xdg_foreign_state: smithay::wayland::xdg_foreign::XdgForeignState::new::<Self>(
                &display,
            ),
            wlr_data_control_state:
                smithay::wayland::selection::wlr_data_control::DataControlState::new::<Self, _>(
                    &display,
                    Some(&primary_selection_state),
                    |_| true,
                ),
            ext_data_control_state:
                smithay::wayland::selection::ext_data_control::DataControlState::new::<Self, _>(
                    &display,
                    Some(&primary_selection_state),
                    |_| true,
                ),
            single_pixel_buffer_state:
                smithay::wayland::single_pixel_buffer::SinglePixelBufferState::new::<Self>(&display),
            content_type_state: smithay::wayland::content_type::ContentTypeState::new::<Self>(
                &display,
            ),
            virtual_keyboard_state:
                smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState::new::<Self, _>(
                    &display,
                    |_| true,
                ),
            text_input_state: smithay::wayland::text_input::TextInputManagerState::new::<Self>(
                &display,
            ),
            input_method_state: smithay::wayland::input_method::InputMethodManagerState::new::<
                Self,
                _,
            >(&display, |_| true),
            session_lock_state: smithay::wayland::session_lock::SessionLockManagerState::new::<
                Self,
                _,
            >(&display, |_| true),
            locked: false,
            lock_surfaces: Vec::new(),
            locker: None,
            idle_notifier: None,
            syncobj_state: None,
            // Small protocols programs look for: touchpad gestures passed
            // on, a window's see-through as a whole, its bell, its icon
            // and its tag (the last three are taken and not yet used).
            output_management: Default::default(),
            toplevel_management: Default::default(),
            workspace_management: Default::default(),
            image_captures: Vec::new(),
            pointer_watches: Vec::new(),
            pointer_target: None,
            input_method_of: None,
            dragged: None,
            xdnd_out: None,
            x_drag: Default::default(),
            dnd_on: false,
            drag_asked: None,
            sharing: Default::default(),
            capture_changes: std::collections::HashMap::new(),
            capture_changes_everywhere: 0,
            icon_serial: 0,
            pointer_gestures_state: smithay::wayland::pointer_gestures::PointerGesturesState::new::<
                Self,
            >(&display),
            alpha_modifier_state: smithay::wayland::alpha_modifier::AlphaModifierState::new::<Self>(
                &display,
            ),
            system_bell_state: smithay::wayland::xdg_system_bell::XdgSystemBellState::new::<Self>(
                &display,
            ),
            toplevel_tag_state: smithay::wayland::xdg_toplevel_tag::XdgToplevelTagManager::new::<
                Self,
            >(&display),
            // 1: CLOCK_MONOTONIC.
            presentation_state: smithay::wayland::presentation::PresentationState::new::<Self>(
                &display, 1,
            ),
            pointer_exact: (0.0, 0.0),
            primary_selection_state,
            idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(
                &display,
            ),
            idle_inhibitors: Default::default(),
            displays_off: None,
            idled_off: false,
            last_activity: std::time::Instant::now(),
            foreign_toplevels:
                smithay::wayland::foreign_toplevel_list::ForeignToplevelListState::new::<Self>(
                    &display,
                ),
            foreign_handles: Default::default(),
            activation_state: smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(
                &display,
            ),
            layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState::new::<Self>(
                &display,
            ),
            layer_focus: None,
            xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(
                &display,
            ),
            xwm: None,
            x_display: None,
            x_client: None,
            x_scale: 1.0,
            loop_handle: None,
            x11_overrides: Vec::new(),
            popups: PopupManager::default(),
            running: true,
            seat_state,
            seat,
            display,
            config,
            desktop,
            core,
            outputs: Vec::new(),
            start_time: std::time::Instant::now(),
            rules,
            host: gevrek_host::wm::HostState {
                events: node_events,
                ..Default::default()
            },
            bindings: Vec::new(),
            paused: false,
            suppressed_keys: Default::default(),
            pressed_with: Default::default(),
            buttons_pressed_with: Default::default(),
            pending_bindings: Vec::new(),
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            draw_own_cursor: false,
            arrow: smithay::backend::renderer::element::memory::MemoryRenderBuffer::from_slice(
                &crate::cursor::arrow(),
                smithay::backend::allocator::Fourcc::Argb8888,
                (crate::cursor::ARROW_WIDTH, crate::cursor::ARROW_HEIGHT),
                1,
                Transform::Normal,
                None,
            ),
            vt_request: None,
            vt_switching: false,
            swipe: None,
            headless: false,
            virtual_outputs: 0,
            drawn_gradient_angle: 0.0,
            wallpapers: Default::default(),
            layer_shadows: Default::default(),
            backdrop_id: smithay::backend::renderer::element::Id::new(),
            backdrop_commit: Default::default(),
            captures: Vec::new(),
            live_outputs: false,
            vrr_on: Default::default(),
            night_light_impossible: false,
            night_preview: None,
            frames_shown: Default::default(),
            lock_change: None,
            veiled: false,
            vignette: crate::render::vignette(),
            lock_warning: 0,
            lock_warning_id: smithay::backend::renderer::element::Id::new(),
            lock_warning_commit: Default::default(),
            output_rescan: false,
            output_trial: None,
            virtual_sizes: Vec::new(),
            hide_pointer: false,
            capture_pass: false,
            backdrop_pass: false,
            soft: Default::default(),
            picking: None,
            pick_button: None,
            pick_cancel_asked: false,
            pick_escape: None,
            xwayland_wanted: false,
            xwayland_restarts: Vec::new(),
            swipe_pull: None,
            paced_wake: false,
            frames_told: std::time::Instant::now(),
            frames_owed: false,
            workspace_overview: None,
            overview_leave_asked: false,
            overview_escape: None,
            overview_button: None,
            swipe_forwarded: false,
            pending_program: None,
            ends_with_program: false,
            host_pace: Default::default(),
            input_devices: Vec::new(),
            applied_keymap: None,
            session_mode: false,
            rescue_pressed: None,
            drag_icon: None,
            drag_icon_offset: (0, 0),
            cursors,
            injected_time: 0,
            gesture_progress: None,
            gestures_available: false,
            output_globals: Default::default(),
            started: false,
            drag: None,
            held: Vec::new(),
            hot_corners: Default::default(),
            window_overview: Default::default(),
            pending_holds: Vec::new(),
            suppressed_buttons: Default::default(),
            popup_grab: None,
            surface_outputs: Default::default(),
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            dmabuf_global: None,
            dmabuf_node: None,
            dmabuf_import: None,
            dmabuf_used: false,
            capture_card: None,
            screenshot: None,
            cascade: 0,
            next_output: 0,
        }
    }

    /// Announce an output to clients and to the engine. `rect` is its place
    /// in the global space, in logical pixels.
    pub fn add_output(&mut self, name: &str, rect: Rect) -> MonitorHandle {
        self.add_scaled_output(name, rect, 1.0)
    }

    /// The scale the configuration asks for the output with this name
    /// (`platform.wayland.output_scales`, else `output_scale`).
    /// How the configuration turns the picture of an output.
    pub fn configured_transform(&self, output: &str) -> gevrek_model::outputs::OutputTransform {
        gevrek_model::outputs::profile_of(&self.config.desktop.outputs, output)
            .map(|profile| profile.transform)
            .unwrap_or_default()
    }

    pub fn configured_scale(&self, output: &str) -> f64 {
        // The output's own entry first, then the older per-system setting.
        gevrek_model::outputs::profile_of(&self.config.desktop.outputs, output)
            .and_then(|profile| profile.scale)
            .filter(|scale| (0.5..=4.0).contains(scale))
            .unwrap_or_else(|| self.config.desktop.platform.wayland.scale_of(output))
    }

    /// Tell each window's surfaces which output they are on and at what
    /// scale to draw, when that changed (a new window, a window moved to
    /// another output, an output gone).
    pub(crate) fn sync_surface_outputs(&mut self) {
        use smithay::wayland::compositor::{
            send_surface_state, with_surface_tree_downward, TraversalAction,
        };
        use smithay::wayland::fractional_scale::with_fractional_scale;
        let ids: Vec<WindowHandle> = self.desktop.windows.keys().copied().collect();
        self.surface_outputs
            .retain(|id, _| self.desktop.windows.contains_key(id));
        for id in ids {
            let Some(window) = self.desktop.windows.get(&id) else {
                continue;
            };
            let rect = window.rect;
            let Some(root) = window.surface.wl_surface() else {
                continue;
            };
            let centre = (rect.x + rect.width / 2, rect.y + rect.height / 2);
            let Some(now) = self
                .desktop
                .outputs
                .iter()
                .find(|output| output.rect.contains_point(centre.0, centre.1))
                .or_else(|| self.desktop.outputs.first())
                .map(|output| (output.id, output.scale))
            else {
                continue;
            };
            let before = self.surface_outputs.get(&id).copied();
            if before == Some(now) {
                continue;
            }
            // The window's surface and those under it (subsurfaces).
            let mut surfaces = Vec::new();
            with_surface_tree_downward(
                &root,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |surface, _, _| surfaces.push(surface.clone()),
                |_, _, _| true,
            );
            let output_of = |id: MonitorHandle| {
                self.outputs
                    .iter()
                    .find(|(other, _)| *other == id)
                    .map(|(_, output)| output)
            };
            for surface in &surfaces {
                if let Some((old, _)) = before.filter(|(old, _)| *old != now.0) {
                    if let Some(output) = output_of(old) {
                        output.leave(surface);
                    }
                }
                if let Some(output) = output_of(now.0) {
                    output.enter(surface);
                }
                smithay::wayland::compositor::with_states(surface, |states| {
                    send_surface_state(surface, states, now.1.ceil() as i32, Transform::Normal);
                    with_fractional_scale(states, |scale| scale.set_preferred_scale(now.1));
                });
            }
            self.surface_outputs.insert(id, now);
        }
        // The shell's surfaces (panels, the launcher, their menus) are
        // told the scale of the output they are on, when it is another
        // than they were told.
        for (_, output) in &self.outputs {
            let wanted = output.current_scale().fractional_scale();
            let map = smithay::desktop::layer_map_for_output(output);
            for layer in map.layers() {
                let mut surfaces = vec![layer.wl_surface().clone()];
                surfaces.extend(
                    PopupManager::popups_for_surface(layer.wl_surface())
                        .map(|(popup, _)| popup.wl_surface().clone()),
                );
                for surface in &surfaces {
                    smithay::wayland::compositor::with_states(surface, |states| {
                        with_fractional_scale(states, |scale| {
                            if scale.preferred_scale() != Some(wanted) {
                                scale.set_preferred_scale(wanted);
                            }
                        });
                    });
                }
            }
        }
    }

    /// As `add_output`, for an output whose device pixels are `scale`
    /// times smaller than logical ones (`rect` is logical).
    pub fn add_scaled_output(&mut self, name: &str, rect: Rect, scale: f64) -> MonitorHandle {
        // No display behind it that says who it is (a window of another
        // session, a picture in memory): it is known by its name.
        let stable_id = gevrek_engine::outputs::edid::stable_id_of_place(name);
        self.add_identified_output(name, rect, scale, stable_id)
    }

    /// As `add_scaled_output`, for a display that says who it is:
    /// `stable_id` is made of its EDID (`MonitorInfo::stable_id`).
    pub fn add_identified_output(
        &mut self,
        name: &str,
        rect: Rect,
        scale: f64,
        stable_id: String,
    ) -> MonitorHandle {
        self.next_output += 1;
        let id = MonitorHandle(self.next_output);
        log::debug!(
            "the output {name} is added as {} at {}x{}+{}+{}, scale {scale}",
            id.0,
            rect.width,
            rect.height,
            rect.x,
            rect.y
        );
        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "gevrek".to_string(),
                model: name.to_string(),
            },
        );
        let mode = Mode {
            size: (
                (f64::from(rect.width) * scale).round() as i32,
                (f64::from(rect.height) * scale).round() as i32,
            )
                .into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Fractional(scale)),
            Some((rect.x, rect.y).into()),
        );
        output.set_preferred(mode);
        let global = output.create_global::<Self>(&self.display);
        self.output_globals.insert(id, global);
        self.outputs.push((id, output));
        self.desktop.outputs.push(WlOutput {
            id,
            name: name.to_string(),
            stable_id,
            rect,
            work: rect,
            usable: rect,
            scale,
            mode: (mode.size.w, mode.size.h),
            transform: gevrek_model::outputs::OutputTransform::Normal,
        });
        self.pack_outputs();
        self.publish_outputs();
        // A display the configuration turns: the picture is turned for it
        // from the start (and is then as wide as the display is high).
        if self.configured_transform(name) != Default::default() {
            self.apply_output_config();
        }
        id
    }

    /// An output changed its size (the nested window was resized).
    /// An output appeared while the WM runs: it gets a workspace like the
    /// outputs that were there at the start.
    pub fn output_added(&mut self, id: MonitorHandle) {
        if !self.started {
            return;
        }
        self.configure_monitors();
        self.apply_workspace_profile_set(|_| true, false);
        self.core.ensure_default_workspaces();
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        self.core.executor.mark_layout_dirty(id);
        self.settle();
    }

    /// An output is gone: clients are told, and the engine moves its
    /// workspaces and windows to an output that is left.
    pub fn remove_output(&mut self, id: MonitorHandle) {
        match self.outputs.iter().find(|(other, _)| *other == id) {
            Some((_, output)) => log::debug!(
                "the output {} ({}) is removed; its layer surfaces are told to close",
                output.name(),
                id.0
            ),
            None => log::debug!("the output {} is removed, and was not known", id.0),
        }
        self.wallpapers.borrow_mut().forget(id);
        self.core.monitor_manager.forget_detail(id);
        if let Some(index) = self.outputs.iter().position(|(other, _)| *other == id) {
            let (_, output) = self.outputs.remove(index);
            // Bars and other shell surfaces on it are told to close: their
            // output is gone.
            for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                layer.layer_surface().send_close();
            }
            if let Some(global) = self.output_globals.remove(&id) {
                self.display.remove_global::<Self>(global);
            }
            drop(output);
        }
        self.desktop.outputs.retain(|output| output.id != id);
        if self.desktop.outputs.is_empty() {
            return;
        }
        self.pack_outputs();
        // The pointer may have been on the output that went.
        let (x, y) = self.pointer_exact;
        let location = self.clamp_to_outputs((x, y).into());
        self.pointer_moved(location, self.start_time.elapsed().as_millis() as u32);
        self.publish_outputs();
        for monitor in self.core.monitor_manager.get_all_monitors() {
            self.core.executor.mark_layout_dirty(monitor.handle);
        }
        self.desktop.needs_redraw = true;
        self.settle();
    }

    /// Put every output where it belongs: where the configuration says
    /// (`desktop.outputs`), the others side by side to the right. After an
    /// output came or went, or the configuration changed: an output that
    /// went must not leave a hole the pointer can be lost in.
    pub(crate) fn pack_outputs(&mut self) {
        let sizes: Vec<(String, i32, i32)> = self
            .desktop
            .outputs
            .iter()
            .map(|output| (output.name.clone(), output.rect.width, output.rect.height))
            .collect();
        let places = gevrek_model::outputs::arrange(&self.config.desktop.outputs, &sizes);
        for (record, (x, y)) in self.desktop.outputs.iter_mut().zip(places) {
            if (record.rect.x, record.rect.y) != (x, y) {
                let moved = (x - record.rect.x, y - record.rect.y);
                record.rect.x = x;
                record.rect.y = y;
                record.work.x += moved.0;
                record.work.y += moved.1;
                record.usable.x += moved.0;
                record.usable.y += moved.1;
                if let Some((_, output)) = self.outputs.iter().find(|(id, _)| *id == record.id) {
                    output.change_current_state(None, None, None, Some((x, y).into()));
                }
            }
        }
    }

    /// The configuration's entries for the displays changed: places, the
    /// primary and scales are applied to the outputs that are there. (Mode
    /// and transform are the hardware backend's, applied when a display is
    /// set up.)
    pub fn apply_output_config(&mut self) {
        let wanted: Vec<_> = self
            .desktop
            .outputs
            .iter()
            .map(|output| {
                (
                    output.id,
                    self.configured_scale(&output.name),
                    self.configured_transform(&output.name),
                )
            })
            .collect();
        for (id, scale, transform) in wanted {
            let Some(record) = self
                .desktop
                .outputs
                .iter_mut()
                .find(|output| output.id == id)
            else {
                continue;
            };
            if (record.scale - scale).abs() < f64::EPSILON && record.transform == transform {
                continue;
            }
            // The device's pixels stay; what a logical pixel is, and which
            // way up the picture is, changes.
            log::debug!(
                "the output {} is set to scale {scale} (was {}), turned {transform:?}",
                record.name,
                record.scale
            );
            let device = record.mode;
            record.scale = scale;
            record.transform = transform;
            if let Some((_, output)) = self.outputs.iter().find(|(other, _)| *other == id) {
                output.change_current_state(
                    None,
                    Some(crate::desktop::turn_of(transform)),
                    Some(Scale::Fractional(scale)),
                    None,
                );
            }
            // Sets the logical size, the work area, and lays out again.
            self.resize_output(id, device.0, device.1);
        }
        self.pack_outputs();
        // (X11 programs draw at the largest of the scales.)
        self.tell_x11_scale();
        let (x, y) = self.pointer_exact;
        let location = self.clamp_to_outputs((x, y).into());
        self.pointer_moved(location, self.start_time.elapsed().as_millis() as u32);
        self.publish_outputs();
        for monitor in self.core.monitor_manager.get_all_monitors() {
            self.core.executor.mark_layout_dirty(monitor.handle);
        }
        self.desktop.needs_redraw = true;
        self.settle();
    }

    /// What a display really is, once the hardware said: its refresh rate
    /// (thousandths of a hertz) and its size in millimetres. Programs are
    /// told (they pace animations and pick sizes by it).
    /// The modes a display offers and the one it is in, for programs that
    /// set displays up (the studio's page).
    pub fn set_output_modes(
        &mut self,
        id: MonitorHandle,
        detail: gevrek_engine::outputs::OutputDetail,
    ) {
        self.core.monitor_manager.set_detail(id, detail);
        self.publish_outputs();
    }

    pub fn describe_output(
        &mut self,
        id: MonitorHandle,
        refresh_millihertz: i32,
        size_mm: (i32, i32),
    ) {
        let Some((_, output)) = self.outputs.iter().find(|(other, _)| *other == id) else {
            return;
        };
        if let Some(mode) = output.current_mode() {
            let mode = Mode {
                size: mode.size,
                refresh: refresh_millihertz.max(1),
            };
            output.change_current_state(Some(mode), None, None, None);
            output.set_preferred(mode);
        }
        let _ = size_mm;
    }

    /// `width` and `height` are device pixels.
    pub fn resize_output(&mut self, id: MonitorHandle, width: i32, height: i32) {
        let (scale, transform) = self
            .desktop
            .outputs
            .iter()
            .find(|output| output.id == id)
            .map_or((1.0, Default::default()), |output| {
                (output.scale, output.transform)
            });
        // (`width` and `height` are the display's own, as it lies; the
        // picture is as wide as a display on its side is high.)
        let upright = crate::desktop::turned_size(transform, (width, height));
        let logical = (
            (f64::from(upright.0) / scale).round() as i32,
            (f64::from(upright.1) / scale).round() as i32,
        );
        if let Some((_, output)) = self.outputs.iter().find(|(other, _)| *other == id) {
            let mode = Mode {
                size: (width, height).into(),
                refresh: 60_000,
            };
            output.change_current_state(Some(mode), None, None, None);
            output.set_preferred(mode);
        }
        if let Some(output) = self
            .desktop
            .outputs
            .iter_mut()
            .find(|output| output.id == id)
        {
            if output.mode != (width, height) {
                log::debug!(
                    "the output {} is now {width}x{height} device pixels ({}x{} logical)",
                    output.name,
                    logical.0,
                    logical.1
                );
            }
            output.mode = (width, height);
            output.rect.width = logical.0;
            output.rect.height = logical.1;
            output.work = output.rect;
        }
        // Bars are arranged for the new size; that sets the work area.
        if let Some((_, output)) = self.outputs.iter().find(|(other, _)| *other == id) {
            let free = {
                let mut map = smithay::desktop::layer_map_for_output(output);
                map.arrange();
                map.non_exclusive_zone()
            };
            if let Some(record) = self
                .desktop
                .outputs
                .iter_mut()
                .find(|record| record.id == id)
            {
                record.work = Rect::new(
                    record.rect.x + free.loc.x,
                    record.rect.y + free.loc.y,
                    free.size.w,
                    free.size.h,
                );
            }
        }
        self.publish_outputs();
        self.core.executor.mark_layout_dirty(id);
        self.settle();
    }

    /// Tell the engine what outputs there are now.
    pub(crate) fn publish_outputs(&mut self) {
        // The primary of the configuration, if it names one that is there.
        let primary = self
            .config
            .desktop
            .outputs
            .iter()
            .filter(|profile| profile.primary && profile.enabled)
            .map(|profile| profile.name.trim().to_string())
            .find(|name| {
                self.desktop
                    .outputs
                    .iter()
                    .any(|output| output.name.eq_ignore_ascii_case(name))
            });
        let mut monitors: Vec<MonitorInfo> = self
            .desktop
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| MonitorInfo {
                handle: output.id,
                name: output.name.clone(),
                device_path: output.name.clone(),
                stable_id: output.stable_id.clone(),
                is_primary: match &primary {
                    Some(name) => output.name.eq_ignore_ascii_case(name),
                    None => index == 0,
                },
                x: output.rect.x,
                y: output.rect.y,
                width: output.rect.width,
                height: output.rect.height,
                // What the bars (layer-shell exclusive zones) leave free.
                work_x: output.work.x,
                work_y: output.work.y,
                work_width: output.work.width,
                work_height: output.work.height,
                dpi_scale: output.scale as f32,
            })
            .collect();
        // Two displays of one model that give no serial number: told apart
        // by their connectors.
        gevrek_engine::outputs::number_same_stable_ids(&mut monitors);
        if self.core.monitor_manager.get_all_monitors().is_empty() {
            self.core.monitor_manager.replace_monitors(monitors);
        } else {
            // What the change of outputs means for the windows (those of an
            // output that went are shown on one that is left, state kept by
            // output follows it) is the engine's to carry out.
            let changes = self.core.monitor_manager.reconcile_with(monitors);
            self.core.executor.apply_monitor_changes(
                &mut self.desktop,
                &self.core.monitor_manager,
                &changes,
            );
        }
        // Other programs' display tools hear of it too.
        self.refresh_output_management();
    }

    /// Run the startup of the configuration: the startup nodes of the graph,
    /// the workspace profiles, and a default workspace where there is none.
    pub fn start(&mut self) -> anyhow::Result<()> {
        self.configure_monitors();
        self.core.snap_zone_manager = gevrek_engine::config_apply::build_snap_zone_manager(
            &self.config.desktop.snap_zones,
            &self.core.monitor_manager,
            |_| false,
        );
        self.started = true;
        self.core.executor.clear_state();
        self.core
            .executor
            .execute_startup(&mut self.desktop, &mut self.core.monitor_manager)?;
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        self.apply_workspace_profile_set(|_| true, true);
        if let Err(error) = self.refresh_bindings() {
            log::error!("bindings: {error}");
        }
        self.core.ensure_default_workspaces();
        // Again, for the workspaces that were only created now.
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        let startup = self.config.desktop.startup.clone();
        self.apply_input_config();
        self.apply_numlock();
        // Programs are started from the host's tick, once clients can
        // connect to everything they need (`clients_may_start`).
        self.host.startup.configure(&startup, false);
        self.emit(
            gevrek_engine::executor::RuntimeEventKind::RuntimeStarted,
            None,
            vec![("config_version".to_string(), self.config.version.clone())],
        );
        self.settle();
        Ok(())
    }

    /// Tell the core what happened to a window, then bring the desktop to
    /// what the core decided.
    pub fn window_event(&mut self, event: WindowEvent) {
        let (kind, window, fields) =
            gevrek_engine::executor::RuntimeEventKind::of_window_event(&event);
        self.emit(kind, Some(window), fields);
        if let Err(error) = self
            .core
            .handle_window_event(&mut self.desktop, event.clone())
        {
            log::warn!("{event:?}: {error}");
        }
        self.settle();
        // Who hears of the event asks for the state: it is to be the
        // state after the event, not the one of a quarter second ago.
        // (Not for every place a dragged window passes.)
        if !matches!(event, WindowEvent::Moved(..) | WindowEvent::Resized(..)) {
            self.host.state_stale.set(true);
        }
    }

    /// Whether programs can be started now: the Wayland socket is up, and
    /// so is Xwayland where X11 programs are to run (a program started
    /// before would not know its display). Not waited for longer than a few
    /// seconds: without Xwayland everything else still works.
    pub fn clients_may_start(&self) -> bool {
        !self.xwayland_wanted
            || self.x_display.is_some()
            || self.start_time.elapsed() > std::time::Duration::from_secs(4)
    }

    /// Carry out a command (a key binding, the CLI, the editor).
    pub fn dispatch(&mut self, command: DispatchCommand) -> anyhow::Result<()> {
        self.core.executor.clear_state();
        let result = self.core.dispatch(&mut self.desktop, command);
        self.settle();
        result
    }

    /// After the core ran: lay out what changed, feed back what the desktop
    /// did in answer (a focus effect is answered with `Focused`), dress the
    /// windows, and hand the keyboard focus to the seat.
    pub fn settle(&mut self) {
        // A focus effect and its `Focused` answer cannot go on forever: the
        // desktop only reports a focus that changed.
        for _ in 0..16 {
            if let Err(error) = self.core.flush_pending_layouts(&mut self.desktop) {
                log::warn!("layout: {error}");
            }
            // A compositor decides the focus itself: when the focused window
            // went away or was hidden, the one used last among those shown
            // gets it (the stacking order is the order of use). Only one
            // of the output the keyboard is on: with nothing shown there
            // (the user went to an output whose workspace is empty) nobody
            // has it, and the window of another output does not get it back.
            let focused = self.desktop.focused;
            if !focused.is_some_and(|id| self.desktop.shown(id)) {
                let keyboard_on = self
                    .core
                    .executor
                    .focused_monitor()
                    .filter(|output| self.core.monitor_manager.get_monitor(*output).is_some());
                let next = self
                    .desktop
                    .visible_in_order()
                    .into_iter()
                    .rev()
                    .find(|id| {
                        let on = self.core.monitor_manager.find_window(*id);
                        match (keyboard_on, on) {
                            (Some(wanted), Some((output, _))) => output == wanted,
                            _ => true,
                        }
                    });
                if next != focused {
                    self.desktop.focused = None;
                    self.desktop.focus_dirty = true;
                    if let Err(error) = self.desktop.apply(&gevrek_engine::Effect::Focus(next)) {
                        log::warn!("focus {next:?}: {error}");
                    }
                }
            }
            let events = std::mem::take(&mut self.desktop.pending_events);
            if events.is_empty() {
                break;
            }
            for event in events {
                // Subscribers hear of it as on every backend.
                let (kind, window, fields) =
                    gevrek_engine::executor::RuntimeEventKind::of_window_event(&event);
                self.emit(kind, Some(window), fields);
                if let Err(error) = self
                    .core
                    .handle_window_event(&mut self.desktop, event.clone())
                {
                    log::warn!("{event:?}: {error}");
                }
            }
        }
        self.core
            .apply_focus_effects(&mut self.desktop, &self.config.desktop);
        // Which windows stand on a strip, and on which display.
        let homes: std::collections::HashMap<_, _> = self
            .core
            .monitor_manager
            .strip_windows()
            .into_iter()
            .collect();
        if homes != self.desktop.homes {
            self.desktop.homes = homes;
            self.desktop.needs_redraw = true;
        }
        // The overview of every workspace shows what was there when it
        // came up: a window more or fewer, or another workspace shown by
        // other means, and it is put away.
        if self.workspace_overview_is_stale() {
            self.leave_workspace_overview(None);
        }
        if std::mem::take(&mut self.desktop.pointer_warped) {
            let (x, y) = self.desktop.pointer;
            let time = self.start_time.elapsed().as_millis() as u32;
            self.pointer_moved((f64::from(x), f64::from(y)).into(), time);
        }
        // Programs are told whether their window is tiled (they then draw
        // no shadow or rounded corners of their own).
        let tiled: Vec<_> = self
            .desktop
            .windows
            .iter()
            .filter(|(_, window)| window.alive && window.mapped)
            .map(|(id, _)| {
                let manager = &self.core.monitor_manager;
                (
                    *id,
                    manager.find_window(*id).is_some() && !manager.is_window_floating(*id),
                )
            })
            .collect();
        for (id, tiled) in tiled {
            self.desktop.set_tiled(id, tiled);
        }
        // What a maximized window fills on each output.
        let usable: Vec<_> = self
            .desktop
            .outputs
            .iter()
            .map(|output| {
                self.core
                    .monitor_manager
                    .get_adjusted_work_area(output.id, &self.desktop)
                    .unwrap_or(output.work)
            })
            .collect();
        for (output, usable) in self.desktop.outputs.iter_mut().zip(usable) {
            output.usable = usable;
        }
        self.sync_focus();
        self.sync_surface_outputs();
        // Other panels' taskbars hear of what changed.
        self.refresh_toplevel_management();
        self.refresh_workspace_management();
    }

    /// Forget the popups that are gone. A menu that held the keyboard and
    /// is gone gives it back to whoever has the focus: smithay would hand
    /// it to what the menu belonged to, which for a panel's menu is the
    /// panel, that never asked for keys.
    pub(crate) fn tidy_popups(&mut self) {
        self.popups.cleanup();
        if !self
            .popup_grab
            .as_ref()
            .is_some_and(|grab| grab.has_ended())
        {
            return;
        }
        self.popup_grab = None;
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.is_grabbed() {
                keyboard.unset_grab(self);
            }
        }
        self.desktop.focus_dirty = true;
        self.settle();
    }

    /// Give the keyboard focus to the window the desktop says has it, and
    /// tell the toplevels which of them is active.
    fn sync_focus(&mut self) {
        // While the session is locked the keyboard is the lock screen's;
        // the window that should have it gets it at the unlock.
        if self.locked {
            return;
        }
        if !std::mem::take(&mut self.desktop.focus_dirty) {
            return;
        }
        let focused = self.desktop.focused;
        if let Some(id) = focused {
            self.desktop.had_focus.insert(id);
        }
        for (id, window) in &self.desktop.windows {
            if !window.alive || !window.surface.ready() {
                continue;
            }
            window
                .surface
                .set_state(StateFlag::Activated, Some(*id) == focused);
        }
        // The focused X11 window is also the top one for the X server.
        if let (Some(WindowSurface::X11(window)), Some(xwm)) = (
            focused
                .and_then(|id| self.desktop.windows.get(&id))
                .map(|window| window.surface.clone()),
            self.xwm.as_mut(),
        ) {
            // (Each time the focus is settled: a failure is said once.)
            static RAISE: crate::failing::Failing =
                crate::failing::Failing::new("raising the focused X11 window for the X server");
            RAISE.note(xwm.raise_window(&window));
        }
        let window_surface = focused
            .and_then(|id| self.desktop.windows.get(&id))
            .and_then(|window| window.surface.key_target());
        // A layer surface that took the keyboard keeps it while it lives.
        if self
            .layer_focus
            .as_ref()
            .is_some_and(|surface| !surface.is_alive())
        {
            self.layer_focus = None;
        }
        let surface = self
            .layer_focus
            .clone()
            .map(KeyTarget::Wl)
            .or(window_surface);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
    }

    /// The user pointed at a window (a click, or the pointer entering it
    /// with focus-follows-mouse), or a new window appeared: it gets the
    /// focus.
    pub fn focus_window(&mut self, id: WindowHandle) {
        if self.desktop.focused == Some(id) {
            return;
        }
        if let Err(error) = self.desktop.apply(&gevrek_engine::Effect::Focus(Some(id))) {
            log::warn!("focus {id:?}: {error}");
        }
        self.settle();
    }

    /// The root surface of a window was committed: its client may have
    /// mapped it, unmapped it, or resized it.
    pub(crate) fn toplevel_committed(&mut self, id: WindowHandle) {
        let Some(window) = self.desktop.windows.get_mut(&id) else {
            return;
        };
        let Some(surface) = window.surface.wl_surface() else {
            return;
        };
        if let WindowSurface::Xdg(toplevel) = &window.surface {
            if !toplevel.is_initial_configure_sent() {
                // The first commit asks for a configure; the client may
                // pick its own size until the WM places it.
                toplevel.send_configure();
                return;
            }
        }
        let buffer_size =
            with_renderer_surface_state(&surface, |state| state.buffer().and(state.surface_size()))
                .flatten();
        log::trace!("commit of {id:?}: a picture of {buffer_size:?}");
        // The visible frame: the geometry the client declared, else the
        // whole surface.
        let geometry = with_states(&surface, |states| {
            states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .geometry
        });
        window.geometry_offset =
            geometry.map_or((0, 0), |geometry| (geometry.loc.x, geometry.loc.y));
        let size = buffer_size.map(|size| geometry.map_or(size, |geometry| geometry.size));
        let was_mapped = window.mapped;
        // While the pointer resizes a window, requests for new sizes go out
        // faster than the client answers them. A commit that answers an
        // older request is drawn, but it is not the client's word on its
        // size: told to the engine as one, it would be asked for again and
        // replace the size the user is dragging to.
        let caught_up = match (&window.surface, window.requested) {
            (WindowSurface::Xdg(toplevel), Some(requested)) => {
                toplevel.current_state().size.map(|size| (size.w, size.h)) == Some(requested)
            }
            _ => true,
        };
        match size {
            Some(size) => {
                let (old_width, old_height) = (window.rect.width, window.rect.height);
                // A tiled window that will not be as small as its tile
                // keeps the tile as its place: it is cut to it, instead of
                // lying over its neighbours and the next display.
                // (With more windows than a display has room for, a tile
                // can come out with no size at all: that is no tile to cut
                // anything to.)
                let tile = window.requested.filter(|(width, height)| {
                    *width > 0
                        && *height > 0
                        && window.tiled
                        && !window.fullscreen
                        && !window.maximized
                });
                let kept = match tile {
                    Some((width, height)) => (size.w.min(width), size.h.min(height)),
                    None => (size.w, size.h),
                };
                window.overflows = kept != (size.w, size.h);
                let size: smithay::utils::Size<i32, smithay::utils::Logical> = kept.into();
                window.rect.width = size.w;
                window.rect.height = size.h;
                window.committed = Some((size.w, size.h));
                window.mapped = true;
                self.desktop.needs_redraw = true;
                if !was_mapped {
                    self.read_metadata(id);
                    if let Some(window) = self.desktop.windows.get_mut(&id) {
                        window.initial_title = window.title.clone();
                        window.initial_app_id = window.app_id.clone();
                    }
                    log::debug!("{} is shown (mapped)", self.window_named(id));
                    self.mapped(id);
                } else if (old_width, old_height) != (size.w, size.h) && caught_up {
                    self.window_event(WindowEvent::Resized(id, size.w, size.h));
                    if self.is_shortcut_list(id) {
                        self.center_shortcut_list(id);
                    }
                }
            }
            None if was_mapped => {
                window.mapped = false;
                self.desktop.needs_redraw = true;
                log::debug!(
                    "{} is hidden by its program (unmapped)",
                    self.window_named(id)
                );
                self.window_event(WindowEvent::Unmapped(id));
            }
            None => {}
        }
    }

    /// A window appeared: the core adopts it, it is dressed as the
    /// configuration says, and it gets the focus.
    fn mapped(&mut self, id: WindowHandle) {
        // A program that asked for fullscreen before its window was shown
        // (as games do): the request found a window with no place yet, so
        // it is taken back here and made again once the core has the
        // window, as for one that is there already.
        let early_fullscreen = self.desktop.windows.get_mut(&id).and_then(|window| {
            let WindowSurface::Xdg(surface) = &window.surface else {
                return None;
            };
            if !window.fullscreen {
                return None;
            }
            window.fullscreen = false;
            window.restore = None;
            Some(surface.clone())
        });
        // A place to start from: the middle of the output the pointer is
        // on, a little further down for each window already there. A tiled
        // window is moved at once; a floating one stays.
        // (Or the display a launcher said its application is to open on,
        // if it said so a moment ago: the first new window takes that.)
        let expected = self
            .expected_window
            .take()
            .filter(|(_, since)| since.elapsed() < EXPECTED_WINDOW_HOLDS)
            .filter(|_| !self.is_shortcut_list(id))
            .and_then(|(name, _)| {
                self.desktop
                    .outputs
                    .iter()
                    .find(|output| output.name.eq_ignore_ascii_case(&name))
                    .map(|output| output.id)
            });
        let screen = expected
            .or_else(|| self.desktop.output_at_cursor())
            .and_then(|output| self.desktop.output_rectangle(output));
        // (A window that floats and has a place of its own starts there:
        // where its X11 program put it, or over the window it belongs to.)
        let own = self
            .own_starting_place(id)
            .filter(|_| expected.is_none() && !self.is_shortcut_list(id));
        if let Some(window) = self.desktop.windows.get_mut(&id) {
            window.placed_itself = own.is_some();
        }
        if let (Some((x, y)), Some(window)) = (own, self.desktop.windows.get_mut(&id)) {
            window.rect.x = x;
            window.rect.y = y;
        } else if let (Some(screen), Some(window)) = (screen, self.desktop.windows.get_mut(&id)) {
            let step = 32 * (self.cascade % 8) as i32;
            self.cascade += 1;
            window.rect.x = screen.x + ((screen.width - window.rect.width) / 2).max(0) + step;
            window.rect.y = screen.y + ((screen.height - window.rect.height) / 2).max(0) + step;
        }
        self.announce_toplevel(id);
        self.window_event(WindowEvent::Created(id));
        // (An X11 window may be fullscreen from the start, in ways for
        // which no request comes.)
        self.x11_fullscreen_from_the_start(id);
        if let Some(surface) = early_fullscreen {
            log::info!("A window is fullscreen from the start");
            self.client_state_request(
                surface,
                gevrek_engine::contract::StateChange::Fullscreen(true),
            );
        }
        self.core
            .apply_appearance_to_window(&mut self.desktop, &self.config.desktop, id);
        if self.desktop.shown(id) {
            self.focus_window(id);
        }
        // An X11 window that was given no tile (it floats) has not been
        // told where it is: its program would reckon from where it last
        // was to the X server, and open its menus and tooltips there.
        if let Some(record) = self.desktop.windows.get(&id) {
            if let WindowSurface::X11(window) = &record.surface {
                let at = window.geometry().loc;
                if !record.fullscreen
                    && (at.x, at.y) != (record.rect.x, record.rect.y)
                    && record.rect.width > 0
                    && record.rect.height > 0
                {
                    record.surface.request_rect(record.rect);
                }
            }
        }
        // After the core has seen the window: whatever it did with it, the
        // list of shortcuts ends in the middle.
        if self.is_shortcut_list(id) {
            self.center_shortcut_list(id);
        }
    }

    /// Read the title and application id the client set. Returns whether
    /// either changed.
    fn read_metadata(&mut self, id: WindowHandle) -> bool {
        let Some(window) = self.desktop.windows.get_mut(&id) else {
            return false;
        };
        let (title, app_id) = match &window.surface {
            WindowSurface::Xdg(toplevel) => with_states(toplevel.wl_surface(), |states| {
                let data = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|data| data.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
                match data {
                    Some(data) => (
                        data.title.clone().unwrap_or_default(),
                        data.app_id.clone().unwrap_or_default(),
                    ),
                    None => (String::new(), String::new()),
                }
            }),
            // The class of an X11 window is what rules match as the class.
            WindowSurface::X11(window) => (window.title(), window.class()),
        };
        let changed = window.title != title || window.app_id != app_id;
        window.title = title;
        window.app_id = app_id;
        changed
    }

    pub(crate) fn metadata_changed(&mut self, surface: &WindowSurface) {
        let Some(id) = self.desktop.id_of(surface) else {
            return;
        };
        let mapped = self
            .desktop
            .windows
            .get(&id)
            .is_some_and(|window| window.mapped);
        if self.read_metadata(id) && mapped {
            self.update_toplevel(id);
            self.window_event(WindowEvent::MetadataChanged(id));
            self.refresh_toplevel_management();
        }
    }
}

impl State {
    /// A window as the log names it: its number, its kind, its
    /// application id and its title.
    pub(crate) fn window_named(&self, id: WindowHandle) -> String {
        match self.desktop.windows.get(&id) {
            Some(window) => format!(
                "{} window {} ({:?}, {:?})",
                match window.surface {
                    WindowSurface::Xdg(_) => "the",
                    WindowSurface::X11(_) => "the X11",
                },
                id.0,
                window.app_id,
                window.title,
            ),
            None => format!("the window {} (no longer known)", id.0),
        }
    }

    /// The client a surface belongs to, as the log names it.
    pub(crate) fn client_named(&self, surface: &WlSurface) -> String {
        match self.display.get_client(surface.id()) {
            Ok(client) => named_client(&client),
            Err(_) => "a client that has gone".to_string(),
        }
    }

    /// A surface as the log names it: the window it is, the layer
    /// surface it is (by its namespace), or else its client.
    pub(crate) fn surface_named(&self, surface: &WlSurface) -> String {
        if let Some(id) = self.desktop.id_of_surface(surface) {
            return self.window_named(id);
        }
        let layer = self.outputs.iter().find_map(|(_, output)| {
            smithay::desktop::layer_map_for_output(output)
                .layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                .map(|layer| (layer.namespace().to_string(), output.name()))
        });
        match layer {
            Some((namespace, output)) => format!("the layer surface {namespace:?} on {output}"),
            None => format!("a surface of {}", self.client_named(surface)),
        }
    }

    /// A window is gone: the core hears of it, and its record is dropped.
    pub(crate) fn window_gone(&mut self, id: WindowHandle) {
        log::debug!("{} is closed", self.window_named(id));
        self.forget_window_icon(id);
        let was_mapped = match self.desktop.windows.get_mut(&id) {
            Some(window) => {
                window.alive = false;
                window.mapped
            }
            None => false,
        };
        if self.desktop.focused == Some(id) {
            self.desktop.focused = None;
            self.desktop.focus_dirty = true;
        }
        if was_mapped {
            self.window_event(WindowEvent::Destroyed(id));
        }
        self.withdraw_toplevel(id);
        self.desktop.untrack_window(id);
        self.desktop.needs_redraw = true;
        self.settle();
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland is a client with its own kind of data.
        if let Some(data) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &data.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("every other client is inserted with ClientState")
            .compositor_state
    }

    /// A picture drawn with the graphics card is shown once the card has
    /// finished drawing it: until then the commit waits. Without this a
    /// frame can reach the screen half drawn (what is seen as flicker in
    /// programs that draw with the card).
    fn new_surface(&mut self, surface: &WlSurface) {
        use smithay::wayland::compositor::{
            add_blocker, add_pre_commit_hook, BufferAssignment, SurfaceAttributes,
        };
        add_pre_commit_hook::<Self, _>(surface, |state, _display, surface| {
            // A program that says itself when its picture is ready
            // (explicit sync) is waited for by that word, not by asking
            // the buffer.
            let acquire = state
                .syncobj_state
                .is_some()
                .then(|| {
                    with_states(surface, |data| {
                        data.cached_state
                            .get::<smithay::wayland::drm_syncobj::DrmSyncobjCachedState>()
                            .pending()
                            .acquire_point
                            .clone()
                    })
                })
                .flatten();
            if let (Some(point), Some(client), Some(handle)) =
                (acquire, surface.client(), state.loop_handle.clone())
            {
                if let Ok((blocker, source)) = point.generate_blocker() {
                    let inserted = handle.insert_source(source, move |_, _, state| {
                        let display = state.display.clone();
                        state
                            .client_compositor_state(&client)
                            .blocker_cleared(state, &display);
                        Ok(())
                    });
                    if inserted.is_ok() {
                        add_blocker(surface, blocker);
                        return;
                    }
                }
            }
            let dmabuf = with_states(surface, |data| {
                data.cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|assignment| match assignment {
                        BufferAssignment::NewBuffer(buffer) => {
                            smithay::wayland::dmabuf::get_dmabuf(buffer).cloned().ok()
                        }
                        _ => None,
                    })
            });
            let (Some(dmabuf), Some(client), Some(handle)) =
                (dmabuf, surface.client(), state.loop_handle.clone())
            else {
                return;
            };
            let Ok((blocker, source)) =
                dmabuf.generate_blocker(smithay::reexports::calloop::Interest::READ)
            else {
                return;
            };
            let inserted = handle.insert_source(source, move |_, _, state| {
                let display = state.display.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &display);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // The picture of what is dragged, moved by its program so that
        // another place of it is under the pointer.
        if self.drag_icon.as_ref() == Some(surface) {
            let moved = buffer_moved(surface);
            self.drag_icon_offset.0 += moved.0;
            self.drag_icon_offset.1 += moved.1;
        }
        if is_sync_subsurface(surface) {
            return;
        }
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.desktop.needs_redraw = true;
        self.popups.commit(surface);
        if root == *surface {
            if let Some(id) = self.desktop.id_of_surface(&root) {
                self.toplevel_committed(id);
            } else {
                self.layer_committed(&root);
                // A surface that takes the keyboard (a launcher) has it
                // from now, not from the next key on: the first key
                // typed into it would go to whoever had it before.
                if self.desktop.focus_dirty {
                    self.settle();
                }
            }
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl State {
    /// A program asks for a state of its own window. It gets it, as when
    /// the user asks with a key; a request before the window is shown is
    /// answered when it is.
    fn client_state_request(
        &mut self,
        surface: ToplevelSurface,
        change: gevrek_engine::contract::StateChange,
    ) {
        let Some(id) = self.desktop.id_of(&WindowSurface::Xdg(surface.clone())) else {
            return;
        };
        // A tiled window has its place in the layout: it is not maximized
        // out of it (one window filling the area is the Monocle layout, or
        // fullscreen). The program is answered that it is not maximized.
        let manager = &self.core.monitor_manager;
        let tiled = manager.find_window(id).is_some() && !manager.is_window_floating(id);
        let refused = tiled && change == gevrek_engine::contract::StateChange::Maximized(true);
        if !refused {
            if let Err(error) = self
                .desktop
                .apply(&gevrek_engine::Effect::State { id, change })
            {
                log::warn!("{change:?}: {error}");
            }
        }
        // The protocol wants an answer to every such request, also when
        // nothing changed.
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
        // Leaving a state puts a tiled window back into the layout.
        if let Some(monitor) = self.desktop.output_of(id) {
            self.core.executor.mark_layout_dirty(monitor);
        }
        self.settle();
    }

    /// Where a popup goes, relative to its parent: where its program asks,
    /// moved by the program's own rules (flip, slide, resize) so that all of
    /// it is on the output its window is on.
    fn popup_geometry(
        &self,
        surface: &PopupSurface,
        positioner: &PositionerState,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        use smithay::desktop::{find_popup_root_surface, get_popup_toplevel_coords};
        let popup = PopupKind::Xdg(surface.clone());
        let window = find_popup_root_surface(&popup)
            .ok()
            .and_then(|root| self.desktop.id_of_surface(&root))
            .and_then(|id| self.desktop.windows.get(&id));
        let Some(window) = window else {
            // The popup of a layer surface, or of a window that is gone.
            return positioner.get_geometry();
        };
        // The output the middle of the window is on.
        let centre = (
            window.rect.x + window.rect.width / 2,
            window.rect.y + window.rect.height / 2,
        );
        let output = self
            .desktop
            .outputs
            .iter()
            .find(|output| {
                let rect = output.rect;
                centre.0 >= rect.x
                    && centre.0 < rect.x + rect.width
                    && centre.1 >= rect.y
                    && centre.1 < rect.y + rect.height
            })
            .or(self.desktop.outputs.first());
        let Some(output) = output else {
            return positioner.get_geometry();
        };
        // The output as the popup's parent sees it: the parent's own origin
        // is the window's place plus the places of the popups in between.
        let parent = get_popup_toplevel_coords(&popup);
        let target = smithay::utils::Rectangle::new(
            (
                output.rect.x - window.rect.x - parent.x,
                output.rect.y - window.rect.y - parent.y,
            )
                .into(),
            (output.rect.width, output.rect.height).into(),
        );
        (*positioner).get_unconstrained_geometry(target)
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let pid = surface
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display).ok())
            .and_then(|credentials| u32::try_from(credentials.pid).ok());
        let who = self.client_named(surface.wl_surface());
        let id = self.desktop.track(WindowSurface::Xdg(surface), pid);
        log::debug!("{who} made the window {} (not shown until it draws)", id.0);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.desktop.id_of(&WindowSurface::Xdg(surface)) {
            self.window_gone(id);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(&WindowSurface::Xdg(surface));
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(&WindowSurface::Xdg(surface));
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        match surface.get_parent_surface() {
            Some(parent) => log::debug!("a popup of {} is opened", self.surface_named(&parent)),
            None => log::debug!(
                "a popup of {} is opened, waiting for its layer surface",
                self.client_named(surface.wl_surface())
            ),
        }
        let geometry = self.popup_geometry(&surface, &positioner);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        // One without a parent is waiting to be given a layer surface,
        // and is placed (and told so) when it has it.
        if surface.get_parent_surface().is_some() {
            if let Err(error) = surface.send_configure() {
                log::debug!("popup configure: {error:?}");
            }
        }
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            log::debug!("popup: {error:?}");
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        log::debug!(
            "a popup of {} is closed",
            self.client_named(surface.wl_surface())
        );
    }

    /// A menu asks for the keyboard and for the pointer to stay with its
    /// program: keys go to the menu, a press on anything of another program
    /// (or on nothing) dismisses it.
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        use smithay::desktop::{
            find_popup_root_surface, PopupKeyboardGrab, PopupPointerGrab, PopupUngrabStrategy,
        };
        use smithay::input::pointer::Focus;
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        // While the session is locked no program takes the keyboard by
        // a menu of its own: what is typed then is a password.
        if self.locked {
            log::debug!("a popup's grab is refused: the session is locked");
            surface.send_popup_done();
            return;
        }
        let popup = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&popup) else {
            log::debug!("a popup's grab is refused: it belongs to no surface any more");
            return;
        };
        let root_named = self.surface_named(&root);
        let mut grab = match self
            .popups
            .grab_popup(KeyTarget::Wl(root), popup, &seat, serial)
        {
            Ok(grab) => grab,
            Err(error) => {
                log::debug!("popup grab refused: {error:?}");
                return;
            }
        };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                log::debug!("a popup's grab is refused: the keyboard is held by another grab");
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                log::debug!("a popup's grab is refused: the pointer is held by another grab");
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
        log::debug!("a popup of {root_named} has the keyboard and the pointer");
        self.popup_grab = Some(grab);
    }

    /// A program asks for the whole output (a video, a game, F11).
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        self.client_state_request(
            surface,
            gevrek_engine::contract::StateChange::Fullscreen(true),
        );
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.client_state_request(
            surface,
            gevrek_engine::contract::StateChange::Fullscreen(false),
        );
    }

    /// A program asks to fill the usable area (a double click on its own
    /// title bar).
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.client_state_request(
            surface,
            gevrek_engine::contract::StateChange::Maximized(true),
        );
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.client_state_request(
            surface,
            gevrek_engine::contract::StateChange::Maximized(false),
        );
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.client_state_request(
            surface,
            gevrek_engine::contract::StateChange::Minimized(true),
        );
    }

    /// A client asks to be moved (the user pressed on its own title bar).
    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        if !self.desktop.primary_button_down || self.drag.is_some() {
            return;
        }
        if let Some(id) = self.desktop.id_of(&WindowSurface::Xdg(surface)) {
            self.begin_drag(id, None, 0x110);
        }
    }

    /// A client asks to be resized from one of its edges or corners.
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        if !self.desktop.primary_button_down || self.drag.is_some() {
            return;
        }
        // The protocol numbers edges as bits: top 1, bottom 2, left 4, right 8.
        let bits = edges as u32;
        if let Some(id) = self.desktop.id_of(&WindowSurface::Xdg(surface)) {
            self.begin_drag(id, Some((bits & 4 != 0, bits & 1 != 0)), 0x110);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let geometry = self.popup_geometry(&surface, &positioner);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = KeyTarget;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, _leds: smithay::input::keyboard::LedState) {
        self.update_keyboard_leds();
    }

    /// The client that has the keyboard is the one that may read and set
    /// the clipboard.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyTarget>) {
        use smithay::wayland::seat::WaylandFocus;
        match focused {
            Some(KeyTarget::Wl(surface)) => {
                log::debug!("the keyboard goes to {}", self.surface_named(surface))
            }
            Some(KeyTarget::X11(window)) => match self
                .desktop
                .id_of(&WindowSurface::X11(window.clone()))
            {
                Some(id) => log::debug!("the keyboard goes to {}", self.window_named(id)),
                None => log::debug!("the keyboard goes to an X11 window ({:?})", window.class()),
            },
            None => log::debug!("the keyboard goes to no surface"),
        }
        let client = focused
            .and_then(|target| target.wl_surface())
            .and_then(|surface| self.display.get_client(surface.id()).ok());
        smithay::wayland::selection::data_device::set_data_device_focus(
            &self.display,
            seat,
            client.clone(),
        );
        smithay::wayland::selection::primary_selection::set_primary_focus(
            &self.display,
            seat,
            client,
        );
    }

    /// The client under the pointer says what the pointer looks like.
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // (While a window is being picked the pointer is a cross: what
        // the client says is what it looks like afterwards.)
        if let Some(afterwards) = &mut self.picking {
            *afterwards = image;
            return;
        }
        self.cursor_status = image;
        self.desktop.needs_redraw = true;
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    /// A Wayland client put something on the clipboard: X11 programs are
    /// offered it too.
    fn new_selection(
        &mut self,
        target: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        match &source {
            Some(source) => log::debug!(
                "a Wayland client put something on the {target:?} selection ({} kinds of data)",
                source.mime_types().len()
            ),
            None => log::debug!("the {target:?} selection is emptied"),
        }
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(error) = xwm.new_selection(target, source.map(|source| source.mime_types()))
            {
                log::warn!("clipboard to X11: {error}");
            }
            // That only queued the request that makes gevrek the owner of
            // the X11 selection: it reached the X server with whatever was
            // sent next, which could be much later, and an X11 program that
            // read the clipboard before then found nobody owning it and got
            // nothing. A question to the server sends what is queued (the
            // window manager's connection has no flush of its own to call);
            // the answer is of no interest.
            let _ = xwm.get_randr_primary_output();
        }
    }

    /// A Wayland client asks for what an X11 program put on the clipboard.
    fn send_selection(
        &mut self,
        target: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let (Some(xwm), Some(handle)) = (self.xwm.as_mut(), self.loop_handle.clone()) {
            if let Err(error) = xwm.send_selection(target, mime_type, fd, handle) {
                log::warn!("clipboard from X11: {error}");
            }
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }

    /// What a drag does where it is: what the keys held ask for (or the
    /// X11 program whose drag it is), where both sides allow it
    /// (`dnd.rs`).
    fn action_choice(
        &mut self,
        available: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
        preferred: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    ) -> smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction {
        crate::dnd::choose(available, preferred, self.drag_wish())
    }
}

/// How far a surface's program moved its picture with its last commit
/// (`wl_surface.offset`, or the place given with the buffer): taken, so
/// that it counts once.
fn buffer_moved(surface: &WlSurface) -> (i32, i32) {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<smithay::wayland::compositor::SurfaceAttributes>()
            .current()
            .buffer_delta
            .take()
    })
    .map_or((0, 0), |moved| (moved.x, moved.y))
}

impl smithay::wayland::shell::xdg::dialog::XdgDialogHandler for State {
    /// (Whether a window floats is decided when it is first shown, from
    /// what it has said of itself until then.)
    fn modal_changed(&mut self, _toplevel: ToplevelSurface, _is_modal: bool) {}
}
smithay::delegate_xdg_dialog!(State);

impl XdgDecorationHandler for State {
    /// gevrek draws the borders, and tiled windows have no use for a title
    /// bar of their own: clients are asked not to draw decorations.
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        self.new_decoration(toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.new_decoration(toplevel);
    }
}

impl ClientDndGrabHandler for State {
    /// A drag began: what is dragged may have a picture of its own, which
    /// follows the pointer until the drop.
    fn started(
        &mut self,
        source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        log::debug!(
            "a drag begins ({} kinds of data, {})",
            source.as_ref().map_or(0, |source| {
                smithay::wayland::selection::data_device::with_source_metadata(source, |data| {
                    data.mime_types.len()
                })
                .unwrap_or(0)
            }),
            if icon.is_some() {
                "with a picture"
            } else {
                "with no picture"
            },
        );
        // (What its program said of its place before the drag began.)
        self.drag_icon_offset = icon.as_ref().map_or((0, 0), buffer_moved);
        self.drag_icon = icon;
        // (Kept for the case that it comes over an X11 window.)
        self.dragged = source;
        self.xdnd_out = None;
        self.dnd_on = true;
        self.drag_asked = crate::dnd::asked_by_keys(self.drag_keys());
        self.desktop.needs_redraw = true;
    }

    fn dropped(&mut self, target: Option<WlSurface>, validated: bool, _seat: Seat<Self>) {
        match &target {
            Some(target) => log::debug!(
                "a drag is let go over {} ({})",
                self.surface_named(target),
                if validated {
                    "it takes the drop"
                } else {
                    "it does not take the drop"
                },
            ),
            None => log::debug!("a drag is let go over nothing that takes it"),
        }
        self.drag_icon = None;
        self.drag_icon_offset = (0, 0);
        self.dragged = None;
        self.dnd_on = false;
        self.drag_asked = None;
        // Let go over an X11 window: dropped there.
        if let Some(mut out) = self.xdnd_out.take() {
            out.tell(if out.over.is_some() {
                crate::xdnd::Told::Dropped
            } else {
                crate::xdnd::Told::Ended
            });
        }
        self.desktop.needs_redraw = true;
    }
}
impl ServerDndGrabHandler for State {
    /// The Wayland window an X11 program's drag is over asks for the
    /// data: fetched from the X11 program.
    fn send(&mut self, mime_type: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>) {
        self.x_drag_fetch(mime_type, fd);
    }

    /// The window under an X11 program's drag and gevrek's own drag
    /// settled on what a drop would do: the X11 program is told.
    fn action(
        &mut self,
        action: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
        _seat: Seat<Self>,
    ) {
        self.x_drag_chose(action);
    }

    fn dropped(&mut self, _seat: Seat<Self>) {
        log::debug!("an X11 program's drag is let go over a Wayland window");
        self.x_drag_let_go(None);
    }

    fn cancelled(&mut self, _seat: Seat<Self>) {
        log::debug!("an X11 program's drag over a Wayland window is cancelled");
        self.x_drag_let_go(Some(false));
    }

    fn finished(&mut self, _seat: Seat<Self>) {
        log::debug!("an X11 program's drag is finished: the Wayland window has the data");
        self.x_drag_done(true);
    }
}
impl OutputHandler for State {}

smithay::delegate_compositor!(State);
smithay::delegate_presentation!(State);
smithay::delegate_drm_syncobj!(State);
smithay::delegate_pointer_gestures!(State);
smithay::delegate_alpha_modifier!(State);
smithay::delegate_xdg_system_bell!(State);
// (xdg-toplevel-icon is not offered: the library ends a program's
// connection when the program lets go of an icon's picture after the icon
// itself, which the protocol allows and Chrome does. gevrek shows no such
// icons yet; when it does, that check has to be mended first.)
smithay::delegate_xdg_toplevel_tag!(State);

impl smithay::wayland::xdg_system_bell::XdgSystemBellHandler for State {
    /// A program rings the bell: there is no sound of gevrek's own yet, so
    /// it is noted and nothing more.
    fn ring(&mut self, _surface: Option<WlSurface>) {
        log::debug!("A program rang the bell");
    }
}

impl smithay::wayland::xdg_toplevel_tag::XdgToplevelTagHandler for State {}

impl smithay::wayland::drm_syncobj::DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut smithay::wayland::drm_syncobj::DrmSyncobjState> {
        self.syncobj_state.as_mut()
    }
}
smithay::delegate_shm!(State);
smithay::delegate_xdg_shell!(State);
smithay::delegate_seat!(State);
smithay::delegate_data_device!(State);
smithay::delegate_output!(State);
smithay::delegate_xdg_decoration!(State);
