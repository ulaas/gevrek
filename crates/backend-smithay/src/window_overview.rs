//! The overview of one workspace's windows (`DispatchCommand::ToggleExpose`),
//! as far as the compositor adds to it.
//!
//! The engine says where each window's picture goes (`expose::arrange`),
//! and the desktop draws the window there, smaller (`WlWindow::overview`).
//! What is here is what stands around the pictures and what chooses among
//! them, as on Windows (`backend-win32/src/dwm/overview.rs`): each
//! window's title under its picture, a frame around the one that is
//! pointed at, the arrow keys and Tab to go from one to another, Enter to
//! go to it, Escape to put the overview away.
//!
//! What is drawn is made when the overview comes up and when a picture's
//! place or a window's title is another (`sync_window_overview`), not at
//! every frame.

use crate::state::State;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::command::DispatchCommand;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::Id;

/// How far the frame of the picture that is pointed at stands out.
pub const MARK: i32 = 4;

/// One window of the overview: where its picture is, and its title as
/// something to draw.
pub struct Shown {
    pub window: WindowHandle,
    pub rect: Rect,
    pub title: String,
    pub label: MemoryRenderBuffer,
    pub label_size: (i32, i32),
    /// The names its frame's four sides are drawn under.
    pub ids: [Id; 4],
}

/// The overview's own part, kept in [`State`].
#[derive(Default)]
pub struct WindowOverview {
    pub shown: Vec<Shown>,
    /// The one that is pointed at, by the pointer or the keys.
    pub marked: Option<WindowHandle>,
    /// The keys that went down while it was up: their release is the
    /// overview's too, though it may be gone by then.
    pub(crate) held: std::collections::HashSet<u32>,
    /// A key of the overview's was pressed: carried out after the key.
    pub(crate) asked: Option<String>,
}

/// Whether a key is one the overview answers to.
pub fn is_its_key(name: &str) -> bool {
    matches!(
        name,
        "ESCAPE" | "ENTER" | "SPACE" | "TAB" | "LEFT" | "RIGHT" | "UP" | "DOWN"
    )
}

impl State {
    /// Whether the overview of one workspace's windows is up.
    pub fn window_overview_is_up(&self) -> bool {
        !self.desktop.decorations_visible
            && !self.desktop.overview_all
            && self.workspace_overview.is_none()
            && self
                .desktop
                .windows
                .values()
                .any(|window| window.overview.is_some() && !window.hidden_by_wm)
    }

    /// What is drawn around the pictures is as the pictures are now: made
    /// anew where a place or a title is another, gone when the overview is.
    pub fn sync_window_overview(&mut self) {
        if !self.window_overview_is_up() {
            if !self.window_overview.shown.is_empty() || self.window_overview.marked.is_some() {
                self.window_overview.shown.clear();
                self.window_overview.marked = None;
                self.desktop.needs_redraw = true;
            }
            return;
        }
        let mut there: Vec<(WindowHandle, Rect, String)> = self
            .desktop
            .windows
            .iter()
            .filter(|(_, window)| !window.hidden_by_wm && window.mapped && !window.minimized)
            .filter_map(|(id, window)| Some((*id, window.overview?, window.title.clone())))
            .collect();
        there.sort_by_key(|(id, rect, _)| (rect.y, rect.x, id.0));
        let same = there.len() == self.window_overview.shown.len()
            && there
                .iter()
                .zip(&self.window_overview.shown)
                .all(|((id, rect, title), shown)| {
                    *id == shown.window && *rect == shown.rect && *title == shown.title
                });
        if same {
            return;
        }
        self.window_overview.shown = there
            .into_iter()
            .map(|(window, rect, title)| {
                // (Wider than a narrow picture, so that a title can be read.)
                let (label, label_size) =
                    crate::overview::label(&title, rect.width.max(160), (225, 225, 230));
                Shown {
                    window,
                    rect,
                    title,
                    label,
                    label_size,
                    ids: std::array::from_fn(|_| Id::new()),
                }
            })
            .collect();
        // The window with the keyboard is the one pointed at first: Enter
        // goes back to it, the arrows go on from it.
        let marked = self
            .window_overview
            .marked
            .or(self.desktop.focused)
            .filter(|marked| {
                self.window_overview
                    .shown
                    .iter()
                    .any(|shown| shown.window == *marked)
            });
        self.window_overview.marked = marked;
        self.desktop.needs_redraw = true;
    }

    /// The pointer is over a picture: that one is pointed at.
    pub(crate) fn window_overview_pointed(&mut self) {
        if self.window_overview.shown.is_empty() {
            return;
        }
        let (x, y) = self.desktop.pointer;
        let Some(under) = self.desktop.window_at(x, y) else {
            return;
        };
        if self
            .window_overview
            .shown
            .iter()
            .any(|shown| shown.window == under)
            && self.window_overview.marked != Some(under)
        {
            self.window_overview.marked = Some(under);
            self.desktop.needs_redraw = true;
        }
    }

    /// A key of the overview's: Escape puts it away, the arrows and Tab go
    /// from picture to picture, Enter and the space bar go to the window
    /// that is pointed at.
    pub(crate) fn window_overview_key(&mut self, name: &str) {
        if !self.window_overview_is_up() {
            return;
        }
        let places: Vec<Rect> = self
            .window_overview
            .shown
            .iter()
            .map(|shown| shown.rect)
            .collect();
        let at = self.window_overview.marked.and_then(|marked| {
            self.window_overview
                .shown
                .iter()
                .position(|shown| shown.window == marked)
        });
        let leave = |state: &mut State| {
            if let Err(error) = state.run_command(DispatchCommand::ToggleExpose) {
                log::warn!("the overview was not put away: {error}");
            }
        };
        match name {
            "ESCAPE" => leave(self),
            "ENTER" | "SPACE" => match self.window_overview.marked {
                // (The one with the keyboard already: nothing is chosen
                // anew, the overview is only put away.)
                Some(marked) if self.desktop.focused != Some(marked) => {
                    self.focus_window(marked);
                    self.settle();
                }
                _ => leave(self),
            },
            _ if places.is_empty() => {}
            _ => {
                let next = match (at, name) {
                    (None, _) => 0,
                    (Some(from), "TAB") => (from + 1) % places.len(),
                    (Some(from), key) => {
                        let (across, down) = match key {
                            "LEFT" => (-1, 0),
                            "RIGHT" => (1, 0),
                            "UP" => (0, -1),
                            _ => (0, 1),
                        };
                        gevrek_engine::expose::towards(&places, from, across, down).unwrap_or(from)
                    }
                };
                self.window_overview.marked = Some(self.window_overview.shown[next].window);
                self.desktop.needs_redraw = true;
            }
        }
    }
}
