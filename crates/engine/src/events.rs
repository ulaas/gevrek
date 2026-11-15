//! What the backend observed about a window. A backend's host feeds these
//! to `core::WmCore::handle_window_event`.

use crate::WindowHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEvent {
    Created(WindowHandle),
    Destroyed(WindowHandle),
    Focused(WindowHandle),
    Moved(WindowHandle, i32, i32),
    Resized(WindowHandle, i32, i32),
    // User drag events - reliable detection via Windows messages
    DragStarted(WindowHandle), // User started dragging/resizing (WM_ENTERSIZEMOVE)
    DragEnded(WindowHandle),   // User finished dragging/resizing (WM_EXITSIZEMOVE)
    // Window state change events
    Minimized(WindowHandle), // Window was minimized
    Restored(WindowHandle),  // Window was restored from minimized state
    /// Title/class-accessible metadata changed; dynamic rules must be re-evaluated.
    MetadataChanged(WindowHandle),
    /// The window was hidden (`EVENT_OBJECT_HIDE`). Hides the WM made itself
    /// are filtered by the runtime; the rest are the app's own (e.g. minimize
    /// to tray), and the window leaves the layout until it is shown again.
    Unmapped(WindowHandle),
}
