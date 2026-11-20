//! Showing the desktop (`DispatchCommand::ShowDesktop`), as Windows' own
//! Win+D does: the windows of the workspace every display shows go out of
//! sight and the desktop has the keyboard; asked again, they come back and
//! the window that had the keyboard has it again.
//!
//! The windows are put out of sight the way a change of workspace does it
//! (`hidden_by_wm`, and on Windows the journal of hidden windows, so that
//! a crash shows them again), not minimized: nothing about them changes,
//! so their places, their tiles, the focus within each workspace and a
//! maximized window's state are as they were when they come back. Each is
//! marked (`WindowState::away_for_desktop`) while it is away.
//!
//! It ends by itself, with every window back, when what the person does
//! next is to work with a window again:
//!
//! - another workspace is shown, on any display (`switch_workspace_on`):
//!   the change of workspace then goes on as always;
//! - a window is given the keyboard: a press on its button in the panel,
//!   Alt+Tab, a key that focuses a window (`on_window_focused`, and after
//!   a command that left a window with the keyboard);
//! - a window is brought back from the taskbar after it was minimized;
//! - a new window opens on a workspace a display shows. Windows leaves the
//!   other windows minimized then and shows the new one alone; here the
//!   new window is laid out among the others (it takes a share of their
//!   room), and alone over the desktop it would stand in a tile cut for a
//!   workspace that seems empty. So all of them come back with it;
//! - a display comes or goes: the windows are on other workspaces then;
//! - every hidden window is shown (`RestoreWindows`, gevrek stopping).
//!
//! While the desktop is shown nothing of a shown workspace comes into
//! sight by itself: what a layout pass, a group's tabs, a window moved to
//! a shown workspace or one taken back into the tiling would show is put
//! away with the others instead (`apply_changes`, `sync_group_visibility`,
//! `keep_desktop_clear`), and comes back when the desktop ends.

use super::*;

/// What showing the desktop gives back when it ends (the windows that went
/// out of sight are marked in their own state).
#[derive(Debug, Clone, Default)]
pub(super) struct DesktopShown {
    /// The window that had the keyboard: it has it again.
    keyboard: Option<WindowHandle>,
    /// The scratchpads that were shown, by their windows: they are on no
    /// workspace, and come back as they were.
    scratchpads: Vec<WindowHandle>,
}

/// What ended the shown desktop (the `reason` of `desktop.hidden`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DesktopEnds {
    /// It was asked for.
    Command,
    /// Another workspace was shown.
    Workspace,
    /// A window was given the keyboard, or brought back from the taskbar.
    Focus,
    /// A new window opened on a workspace that is shown.
    Window,
    /// A display came or went.
    Displays,
    /// Every hidden window was shown.
    Restore,
}

impl DesktopEnds {
    fn word(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Workspace => "workspace",
            Self::Focus => "focus",
            Self::Window => "window",
            Self::Displays => "displays",
            Self::Restore => "restore",
        }
    }
}

impl Executor {
    /// Whether the desktop is shown now (the windows of the shown
    /// workspaces are out of sight until it ends).
    pub fn desktop_shown(&self) -> bool {
        self.desktop_shown.is_some()
    }

    /// `DispatchCommand::ShowDesktop`: `Some(true)` shows the desktop,
    /// `Some(false)` brings the windows back, `None` does whichever is not
    /// the case now. Asked for what is the case already, nothing happens.
    pub(super) fn show_desktop(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        on: Option<bool>,
    ) -> Result<()> {
        let shown = self.desktop_shown.is_some();
        match on.unwrap_or(!shown) {
            true if !shown => self.put_windows_away(window_manager, monitor_manager),
            false if shown => {
                self.end_show_desktop(window_manager, monitor_manager, None, true, DesktopEnds::Command);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Every window in sight on the workspace each display shows, and every
    /// scratchpad that is shown, goes out of sight; the desktop gets the
    /// keyboard. The display with the keyboard stays the one it was.
    fn put_windows_away(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> Result<()> {
        // (A window of no workspace that is not a scratchpad, a launcher
        // or a menu, is not one to give the keyboard back to.)
        let keyboard = queries(window_manager).active_window().filter(|window| {
            monitor_manager.find_window(*window).is_some()
                || self.scratchpads.is_scratchpad_window(*window)
        });
        let away = self.windows_in_sight(window_manager, monitor_manager);
        self.put_away(window_manager, &away);
        let mut scratchpads = Vec::new();
        for (id, window, visible) in self.scratchpads.bindings() {
            if !visible || !queries(window_manager).exists(window) {
                continue;
            }
            try_effect(
                window_manager,
                Effect::SetVisible {
                    id: window,
                    visible: false,
                },
            );
            self.scratchpads.set_visible(&id, false);
            scratchpads.push(window);
        }
        let count = away.len() + scratchpads.len();
        self.desktop_shown = Some(DesktopShown {
            keyboard,
            scratchpads,
        });
        // (On Windows the desktop keeps it for a moment against a window
        // the system would give it to by itself as the others go.)
        effect(window_manager, Effect::Focus(None))?;
        log::debug!("The desktop is shown: {count} window(s) out of sight");
        self.emit_desktop_event(
            RuntimeEventKind::DesktopShown,
            [("windows".to_string(), count.to_string())],
        );
        Ok(())
    }

    /// The windows of the workspaces the displays show that are in sight
    /// now, as the desktop says (not those minimized or hidden by their
    /// own programs; a window gevrek counts as hidden that is in sight all
    /// the same is one of them), and not the window that is opening, nor
    /// one already put away whose hiding is still on its way.
    fn windows_in_sight(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> Vec<WindowHandle> {
        let mut windows = Vec::new();
        for monitor in monitor_manager.get_all_monitors() {
            let Some(workspace) = monitor_manager.get_focused_workspace(monitor.handle) else {
                continue;
            };
            for window in workspace.all_windows() {
                let going = self.window_state.get(window).away_for_desktop
                    && window_manager.is_hidden_by_backend(window);
                if going
                    || self.opening == Some(window)
                    || !queries(window_manager).exists(window)
                    || !queries(window_manager).is_visible(window)
                {
                    continue;
                }
                windows.push(window);
            }
        }
        windows
    }

    /// Hide windows as a change of workspace does, and mark them as away
    /// for the desktop.
    fn put_away(&mut self, window_manager: &mut dyn Desktop, windows: &[WindowHandle]) {
        if windows.is_empty() {
            return;
        }
        let mut changes = Changes::default();
        workspaces::hide_windows(&mut self.window_state, windows, &mut changes);
        for window in windows {
            self.window_state.get_mut(*window).away_for_desktop = true;
        }
        // (Hides only: nothing in them is held back while the desktop is
        // shown.)
        self.apply_changes(window_manager, changes);
        for window in windows {
            try_effect(
                window_manager,
                Effect::Border {
                    id: *window,
                    change: BorderChange::Visible(false),
                },
            );
        }
    }

    /// While the desktop is shown, a window of a shown workspace that came
    /// into sight by a way that does not ask (a program that showed its
    /// window again, a command that put one there) is put away with the
    /// others. Run after every layout pass.
    pub(super) fn keep_desktop_clear(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
    ) {
        if self.desktop_shown.is_none() {
            return;
        }
        let sneaked = self.windows_in_sight(window_manager, monitor_manager);
        if !sneaked.is_empty() {
            log::debug!("{sneaked:?} came into sight while the desktop is shown: put away");
            self.put_away(window_manager, &sneaked);
        }
    }

    /// What would show a window while the desktop is shown keeps it away
    /// instead (`apply_changes`, `sync_group_visibility`): it is marked,
    /// and comes back with the others. Returns whether the desktop is
    /// shown (and so whether the window was held back).
    pub(super) fn hold_back_for_desktop(&mut self, window: WindowHandle) -> bool {
        if self.desktop_shown.is_none() || self.opening == Some(window) {
            return false;
        }
        let state = self.window_state.get_mut(window);
        state.hidden_by_wm = true;
        state.away_for_desktop = true;
        true
    }

    /// The desktop is shown no longer: the windows it put away come back,
    /// those that are on a workspace a display shows now and that the
    /// workspace shows (not a tab behind another, not a column out of
    /// view). A window that is on another workspace by now stays out of
    /// sight with it; one that has closed is forgotten. `leaving`: a
    /// display whose workspace is about to be changed, whose windows go on
    /// out of sight with it. `keyboard_back`: the window that had the
    /// keyboard has it again (else what ended it gives the keyboard).
    pub(super) fn end_show_desktop(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        leaving: Option<MonitorHandle>,
        keyboard_back: bool,
        why: DesktopEnds,
    ) {
        let Some(shown) = self.desktop_shown.take() else {
            return;
        };
        let away: HashSet<WindowHandle> = self
            .window_state
            .ids_where(|state| state.away_for_desktop)
            .into_iter()
            .collect();
        for window in &away {
            self.window_state.get_mut(*window).away_for_desktop = false;
        }
        let mut back = Vec::new();
        for monitor in monitor_manager.get_all_monitors() {
            if Some(monitor.handle) == leaving {
                continue;
            }
            let Some(workspace) = monitor_manager.get_focused_workspace(monitor.handle) else {
                continue;
            };
            let coming: Vec<WindowHandle> = workspaces::reveal_of(workspace)
                .shown
                .into_iter()
                .filter(|window| away.contains(window) && queries(window_manager).exists(*window))
                .collect();
            if !coming.is_empty() {
                // (Laid out again once they are in sight: a window closed
                // meanwhile left room that the others were given while
                // they were hidden, and some programs do not move while
                // hidden.)
                self.mark_layout_dirty(monitor.handle);
            }
            back.extend(coming);
        }
        let mut changes = Changes::default();
        workspaces::show_windows(&mut self.window_state, &back, &mut changes);
        self.apply_changes(window_manager, changes);
        for window in &back {
            try_effect(
                window_manager,
                Effect::Border {
                    id: *window,
                    change: BorderChange::Visible(true),
                },
            );
        }
        let mut count = back.len();
        let bindings = self.scratchpads.bindings();
        for window in &shown.scratchpads {
            // (One that was shown and put away again meanwhile, or is
            // another scratchpad's or none's now, stays as it is.)
            let Some((id, _, false)) = bindings.iter().find(|(_, bound, _)| bound == window) else {
                continue;
            };
            if !queries(window_manager).exists(*window) {
                continue;
            }
            try_effect(
                window_manager,
                Effect::SetVisible {
                    id: *window,
                    visible: true,
                },
            );
            self.scratchpads.set_visible(id, true);
            count += 1;
        }
        if keyboard_back {
            let in_sight = |executor: &Self, window: WindowHandle| {
                if !queries(window_manager).exists(window) {
                    return false;
                }
                if let Some((output, index)) = monitor_manager.find_window(window) {
                    return Some(output) != leaving
                        && !executor.window_state.get(window).hidden_by_wm
                        && monitor_manager
                            .get_workspaces(output)
                            .is_some_and(|workspaces| workspaces.focused_idx() == index);
                }
                executor
                    .scratchpads
                    .bindings()
                    .into_iter()
                    .any(|(_, bound, visible)| bound == window && visible)
            };
            let given = match shown.keyboard.filter(|window| in_sight(self, *window)) {
                Some(window) => effect(window_manager, Effect::Focus(Some(window))),
                // (Closed meanwhile, or away with its workspace: the
                // display that has the keyboard gives it as a change of
                // workspace there would.)
                None => match self.focused_monitor {
                    Some(output) => self.focus_on_output(window_manager, monitor_manager, output),
                    None => Ok(()),
                },
            };
            if let Err(error) = given {
                log::debug!("The keyboard was not given back after the desktop: {error:#}");
            }
        }
        log::debug!(
            "The desktop is shown no longer ({}): {count} window(s) back",
            why.word()
        );
        self.emit_desktop_event(
            RuntimeEventKind::DesktopHidden,
            [
                ("windows".to_string(), count.to_string()),
                ("reason".to_string(), why.word().to_string()),
            ],
        );
    }

    /// A scratchpad was shown while the desktop is shown (it is asked for
    /// over the desktop): from now on it is as the person leaves it, and
    /// the end of the desktop does not bring it back again.
    pub(super) fn scratchpad_shown_over_desktop(&mut self, window: WindowHandle) {
        if let Some(shown) = &mut self.desktop_shown {
            shown.scratchpads.retain(|away| *away != window);
        }
    }

    /// Every hidden window was shown (`restore_hidden_windows`): the
    /// desktop is not shown any more, and nothing is marked as away.
    pub(super) fn forget_shown_desktop(&mut self) {
        for window in self.window_state.ids_where(|state| state.away_for_desktop) {
            self.window_state.get_mut(window).away_for_desktop = false;
        }
        if self.desktop_shown.take().is_some() {
            self.emit_desktop_event(
                RuntimeEventKind::DesktopHidden,
                [
                    ("windows".to_string(), "0".to_string()),
                    ("reason".to_string(), DesktopEnds::Restore.word().to_string()),
                ],
            );
        }
    }

    /// Whether a command, given while the desktop is shown, ends it before
    /// it is carried out: it is about a window or a display that is to be
    /// worked with, so the windows are to be seen (and the keyboard is
    /// where it was, for the command to start from). A change of workspace
    /// ends it on its own (`switch_workspace_on`); a command that only
    /// leaves a window with the keyboard ends it afterwards
    /// (`execute_dispatch`).
    pub(super) fn ends_shown_desktop(command: &DispatchCommand) -> bool {
        matches!(
            command,
            DispatchCommand::FocusNext
                | DispatchCommand::FocusPrevious
                | DispatchCommand::FocusLast
                | DispatchCommand::FocusWindow { .. }
                | DispatchCommand::ForceFocusWindow { .. }
                | DispatchCommand::FocusDirection { .. }
                | DispatchCommand::SwapNext
                | DispatchCommand::SwapPrevious
                | DispatchCommand::SwapDirection { .. }
                | DispatchCommand::FocusMonitor { .. }
                | DispatchCommand::FocusMonitorAtCursor
                | DispatchCommand::FocusMonitorWorkspace { .. }
                | DispatchCommand::MoveWorkspaceToMonitor { .. }
                | DispatchCommand::SwapActiveWorkspaces { .. }
                | DispatchCommand::ToggleWindowCentered { .. }
                | DispatchCommand::MoveWindowToMonitorAndFollow { .. }
                | DispatchCommand::MoveToWorkspaceAndFollow { .. }
                | DispatchCommand::MoveWindowToWorkspaceAndFollow { .. }
                | DispatchCommand::NewWorkspace { .. }
                | DispatchCommand::CloseWorkspace
                | DispatchCommand::ManageWindow { .. }
                | DispatchCommand::ToggleExpose
                | DispatchCommand::WorkspaceOverview
                | DispatchCommand::CycleGroupNext
                | DispatchCommand::CycleGroupPrevious
                | DispatchCommand::CycleWindowGroup { .. }
                | DispatchCommand::FocusGroupIndex { .. }
                | DispatchCommand::FocusWindowGroupIndex { .. }
        )
    }

    fn emit_desktop_event<const N: usize>(
        &self,
        kind: RuntimeEventKind,
        fields: [(String, String); N],
    ) {
        let Some(broadcast) = &self.node_event_broadcast else {
            return;
        };
        broadcast(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
            kind,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            window_handle: None,
            fields: fields.into_iter().collect(),
        }));
    }
}
