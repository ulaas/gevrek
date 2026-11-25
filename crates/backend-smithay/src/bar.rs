//! What is left of the bar's own module: the displays' profiles, and the
//! list of shortcuts in the middle of its display.
//!
//! (The bar of before the shell, `gevrek-overlay`, was an ordinary program
//! with a window for each display, which the compositor knew by its title
//! and put in place. It is gone since 2026-11-15: the shell's panel is a
//! layer surface and asks for its own room.)

use crate::state::State;
use gevrek_engine::{Desktop, WindowHandle};

/// What the list of shortcuts calls itself (`gevrek-studio --shortcuts`).
const SHORTCUT_LIST: &str = "gevrek-shortcuts";

impl State {
    /// Whether this window is the list of shortcuts.
    pub(crate) fn is_shortcut_list(&self, id: WindowHandle) -> bool {
        self.desktop
            .windows
            .get(&id)
            .is_some_and(|window| window.alive && window.app_id == SHORTCUT_LIST)
    }

    /// The list of shortcuts is always in the middle of the output the
    /// pointer is on, above the other windows: when it appears, and again
    /// when its size changes. (A Wayland client cannot place its own
    /// window, so the compositor does what the window does itself on
    /// Windows.)
    pub(crate) fn center_shortcut_list(&mut self, id: WindowHandle) {
        let screen = self
            .desktop
            .output_at_cursor()
            .and_then(|output| self.desktop.output_rectangle(output));
        let (Some(screen), Some(window)) = (screen, self.desktop.windows.get_mut(&id)) else {
            return;
        };
        window.always_on_top = true;
        window.drawn = None;
        window.rect.x = screen.x + (screen.width - window.rect.width) / 2;
        window.rect.y = screen.y + (screen.height - window.rect.height) / 2;
        self.desktop.needs_redraw = true;
    }

    /// The monitor and workspace profiles of the configuration, and the
    /// place of the bar.
    pub(crate) fn configure_monitors(&mut self) {
        gevrek_engine::config_apply::configure_monitor_profiles(
            &mut self.core.monitor_manager,
            &self.config.desktop,
        );
        if self
            .core
            .monitor_manager
            .set_panels(&self.config.desktop.shell.panels)
        {
            self.mark_every_layout_dirty();
        }
        // gevrek-shell's panel asks for its own room (a layer surface):
        // none is held for it here.
        self.core.monitor_manager.expect_no_bar();
    }

    fn mark_every_layout_dirty(&mut self) {
        for monitor in self.core.monitor_manager.get_all_monitors() {
            self.core.executor.mark_layout_dirty(monitor.handle);
        }
    }
}
