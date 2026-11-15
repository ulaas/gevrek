//! Carries out engine `Effect`s with the `WindowManager` (docs/PHASE2_PLAN.md,
//! steps 4 and 6). Effects the executor still performs itself (placement,
//! decoration, stackbars, animation) are not handled here yet.

use crate::dwm::WindowManager;
use anyhow::Result;
use gevrek_engine::contract::{
    BorderChange, Effect, PlatformEffect, StackOp, StateChange, WindowsEffect,
};

/// Apply effects in the order the engine emitted them. Failures are logged
/// and skipped: one window refusing must not stop the rest.
pub fn apply(window_manager: &mut WindowManager, effects: &[Effect]) {
    for effect in effects {
        if let Err(error) = apply_one(window_manager, effect) {
            log::debug!("Effect {effect:?} failed: {error:#}");
        }
    }
}

/// Apply one effect and report whether the window accepted it.
pub fn apply_one(window_manager: &mut WindowManager, effect: &Effect) -> Result<()> {
    match effect {
        // Placement only: the executor still applies maximize and fullscreen
        // itself, so `states` is not read here yet.
        Effect::Configure { id, rect, .. } => {
            window_manager.move_and_resize_window(*id, rect.x, rect.y, rect.width, rect.height)
        }
        Effect::SetVisible { id, visible: true } => window_manager.show_window(*id),
        Effect::SetVisible { id, visible: false } => window_manager.hide_window(*id),
        Effect::Focus(Some(id)) => {
            crate::dwm::window_manager::desktop_no_longer_kept();
            window_manager.focus_window(*id)
        }
        Effect::Focus(None) => {
            window_manager.focus_desktop();
            Ok(())
        }
        Effect::Stack { id, op } => match op {
            StackOp::Raise => window_manager.bring_to_front(*id),
            StackOp::Lower => window_manager.send_to_back(*id),
            StackOp::AlwaysOnTop(enabled) => window_manager.set_always_on_top(*id, *enabled),
        },
        Effect::Close(id) => window_manager.close_window(*id),
        Effect::WarpPointer(point) => {
            // SAFETY: SetCursorPos has no preconditions.
            unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(point.x, point.y)? };
            Ok(())
        }
        Effect::DecorationsVisible(true) => {
            window_manager.show_all_borders();
            window_manager.show_all_stackbars();
            Ok(())
        }
        Effect::DecorationsVisible(false) => {
            window_manager.hide_all_borders();
            window_manager.hide_all_stackbars();
            Ok(())
        }
        Effect::HideStackbar(id) => {
            window_manager.hide_stackbar(*id);
            Ok(())
        }
        Effect::Opacity { id, alpha } => window_manager.set_window_alpha(*id, *alpha),
        Effect::Border { id, change } => match change {
            BorderChange::Visible(true) => window_manager.show_border(*id),
            BorderChange::Visible(false) => window_manager.hide_border(*id),
            BorderChange::Remove => window_manager.remove_border(*id),
            BorderChange::Follow => window_manager.update_border_position(*id),
            BorderChange::Focused(focused) => window_manager.set_border_focus(*id, *focused),
        },
        Effect::State { id, change } => match change {
            StateChange::Maximized(true) => window_manager.maximize_window(*id),
            StateChange::Maximized(false) => window_manager.unmaximize_window(*id),
            StateChange::Minimized(true) => window_manager.minimize_window(*id),
            StateChange::Minimized(false) => window_manager.restore_window(*id),
            StateChange::Fullscreen(wanted) => {
                if window_manager.is_window_fullscreen(*id) != *wanted {
                    window_manager.toggle_fullscreen(*id)?;
                }
                Ok(())
            }
        },
        Effect::Platform(PlatformEffect::Windows(effect)) => match effect {
            WindowsEffect::Backdrop { id, backdrop } => {
                window_manager.set_system_backdrop(*id, *backdrop)
            }
            WindowsEffect::Corner { id, corner } => {
                window_manager.set_window_corner_preference(*id, *corner)
            }
            WindowsEffect::Shadow { id, policy } => {
                if !window_manager.set_window_shadow_policy(*id, *policy)? {
                    log::debug!("DWM did not apply shadow policy {policy:?} to {id:?}");
                }
                Ok(())
            }
            WindowsEffect::CaptionButtons { id, shown: true } => {
                window_manager.restore_min_max_buttons(*id)
            }
            WindowsEffect::CaptionButtons { id, shown: false } => {
                window_manager.remove_min_max_buttons(*id)
            }
            WindowsEffect::Title { id, title } => window_manager.set_window_title(*id, title),
            WindowsEffect::Cloak { id, cloaked } => window_manager.cloak_window(*id, *cloaked),
            WindowsEffect::CaptionColor { id, color } => {
                window_manager.set_window_caption_rule_color(*id, *color)
            }
            WindowsEffect::NativeBorderColor { id, color } => {
                window_manager.set_window_border_rule_color(*id, *color)
            }
        },
        other => anyhow::bail!("the Win32 applier cannot apply {other:?} yet"),
    }
}

impl gevrek_engine::contract::WindowQueries for WindowManager {
    fn active_window(&self) -> Option<crate::dwm::WindowHandle> {
        self.get_active_window()
    }

    fn exists(&self, id: crate::dwm::WindowHandle) -> bool {
        self.window_exists(id)
    }

    fn has_title(&self, id: crate::dwm::WindowHandle, title: &str) -> Result<bool, String> {
        self.window_has_title(id, title)
            .map_err(|error| format!("{error:#}"))
    }

    fn position(&self, id: crate::dwm::WindowHandle) -> Option<(i32, i32)> {
        self.get_window_position(id).ok()
    }

    fn size(&self, id: crate::dwm::WindowHandle) -> Option<(i32, i32)> {
        self.get_window_size(id).ok()
    }

    fn title(&self, id: crate::dwm::WindowHandle) -> String {
        self.get_window_title_live(id)
    }

    fn rule_facts(&self, id: crate::dwm::WindowHandle) -> Option<gevrek_engine::rules::WindowInfo> {
        self.rule_window_info(id)
    }

    fn class(&self, id: crate::dwm::WindowHandle) -> String {
        self.get_window_class(id)
    }

    fn exe(&self, id: crate::dwm::WindowHandle) -> String {
        self.get_window_exe(id)
    }

    /// A window's picture on Windows is its program's: the file of the
    /// program, whole, which the system draws the icon of (the name alone,
    /// as `exe` has it, names no file to it). A packaged application's
    /// window is its host's file, and has the host's picture.
    fn icon(&self, id: crate::dwm::WindowHandle) -> String {
        self.get_window_path(id)
    }

    fn path(&self, id: crate::dwm::WindowHandle) -> String {
        self.get_window_path(id)
    }

    fn windows(&self) -> Vec<gevrek_model::window::WindowInfo> {
        self.get_all_windows()
    }

    fn rect(&self, id: crate::dwm::WindowHandle) -> Result<gevrek_model::geometry::Rect, String> {
        self.get_window_rect(id)
            .map_err(|error| format!("{error:#}"))
    }

    fn is_minimized(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_window_minimized(id)
    }

    fn is_maximized(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_window_maximized(id)
    }

    fn is_fullscreen(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_window_fullscreen(id)
    }

    fn is_visible(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_window_visible(id)
    }

    fn is_always_on_top(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_window_pinned(id)
    }

    fn is_tracked(&self, id: crate::dwm::WindowHandle) -> bool {
        self.get_window_info(id).is_some()
    }

    fn smallest_size(&self, id: crate::dwm::WindowHandle) -> Option<(i32, i32)> {
        self.smallest_window_size(id)
    }

    fn pid(&self, id: crate::dwm::WindowHandle) -> Option<u32> {
        self.get_window_pid(id)
    }

    fn initial_title(&self, id: crate::dwm::WindowHandle) -> String {
        self.initial_window_title(id)
    }

    fn initial_class(&self, id: crate::dwm::WindowHandle) -> String {
        self.initial_window_class(id)
    }

    fn hidden_by_app(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_app_unmap(id)
    }

    fn is_decoration(&self, id: crate::dwm::WindowHandle) -> bool {
        self.is_border_window(id)
    }

    fn has_border(&self, id: crate::dwm::WindowHandle) -> bool {
        WindowManager::has_border(self, id)
    }

    fn opacity(&self, id: crate::dwm::WindowHandle) -> u8 {
        self.window_alpha(id)
    }

    fn border_color(&self, id: crate::dwm::WindowHandle) -> gevrek_model::settings::RuleColor {
        self.window_border_color(id)
    }

    fn caption_color(&self, id: crate::dwm::WindowHandle) -> gevrek_model::settings::RuleColor {
        self.window_caption_color(id)
    }

    fn backdrop(&self, id: crate::dwm::WindowHandle) -> crate::dwm::SystemBackdrop {
        self.window_backdrop(id)
    }

    fn corners(&self, id: crate::dwm::WindowHandle) -> crate::dwm::WindowCorner {
        self.window_corner_preference(id)
    }

    fn shadow_policy(&self, id: crate::dwm::WindowHandle) -> gevrek_model::settings::ShadowPolicy {
        self.window_shadow_policy(id)
    }

    fn ignored_by_rules(&self, id: crate::dwm::WindowHandle) -> bool {
        self.should_ignore_window(id)
    }

    fn floats_by_rules(&self, id: crate::dwm::WindowHandle) -> bool {
        self.should_float_window(id)
    }
}

impl gevrek_engine::contract::Desktop for WindowManager {
    fn apply(&mut self, effect: &Effect) -> Result<()> {
        apply_one(self, effect)
    }

    /// Windows opens a program's window where the program last had one,
    /// on whichever display that was: a window that is to be tiled goes to
    /// the display with the keyboard instead.
    fn new_windows_go_where_the_keyboard_is(&self) -> bool {
        true
    }

    fn output_of(&self, id: crate::dwm::WindowHandle) -> Option<crate::dwm::MonitorHandle> {
        use crate::dwm::{HmonitorExt, HwndExt};
        use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
        // The system's desktop (which gets the keyboard when no window is
        // to have it) lies on every display and is on none: which display
        // has the focus is not to be read off it. Unless the person has
        // just pressed a mouse button on it: then it is the display that
        // was pressed on, and that display has the keyboard from then on
        // (the only way to give it to a display with no window on it).
        // SAFETY: plain queries.
        let desktop = unsafe { windows::Win32::UI::WindowsAndMessaging::GetShellWindow() };
        if !desktop.is_invalid() && desktop == id.to_hwnd() {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
            let (x, y) = crate::windows_key::pressed_just_now()?;
            let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
            return (!monitor.is_invalid())
                .then(|| crate::dwm::MonitorHandle::from_hmonitor(monitor));
        }
        // SAFETY: MonitorFromWindow accepts any handle value.
        let monitor = unsafe { MonitorFromWindow(id.to_hwnd(), MONITOR_DEFAULTTONEAREST) };
        (!monitor.is_invalid()).then(|| crate::dwm::MonitorHandle::from_hmonitor(monitor))
    }

    fn output_at_cursor(&self) -> Option<crate::dwm::MonitorHandle> {
        use crate::dwm::HmonitorExt;
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: plain Win32 queries with a valid out pointer.
        unsafe {
            GetCursorPos(&mut point).ok()?;
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            (!monitor.is_invalid()).then(|| crate::dwm::MonitorHandle::from_hmonitor(monitor))
        }
    }

    fn primary_button_down(&self) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
        // SAFETY: GetAsyncKeyState has no preconditions.
        unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
    }

    fn apply_animated_rectangles(
        &mut self,
        operations: &[(crate::dwm::WindowHandle, gevrek_model::geometry::Rect)],
        scope: crate::dwm::TransitionScope,
    ) -> Result<()> {
        WindowManager::apply_animated_rectangles(self, operations, scope)
    }

    fn workspace_gives_way(&mut self) {
        WindowManager::workspace_gives_way(self);
    }

    fn begin_workspace_transition(
        &mut self,
        operations: &[(crate::dwm::WindowHandle, gevrek_model::geometry::Rect)],
        offset_x: i32,
        offset_y: i32,
    ) -> Result<bool> {
        WindowManager::begin_workspace_transition(self, operations, offset_x, offset_y)
    }

    fn start_configured_window_enter(&mut self, id: crate::dwm::WindowHandle) -> Result<bool> {
        WindowManager::start_configured_window_enter(self, id)
    }

    fn start_window_animation(
        &mut self,
        id: crate::dwm::WindowHandle,
        animation_type: crate::dwm::DwmAnimationType,
        duration_ms: u32,
    ) -> Result<bool> {
        WindowManager::start_window_animation(self, id, animation_type, duration_ms)
    }

    fn is_animation_event_suppressed(&self, id: crate::dwm::WindowHandle) -> bool {
        WindowManager::is_animation_event_suppressed(self, id)
    }

    fn preferred_window_size(&self, id: crate::dwm::WindowHandle) -> Option<(i32, i32)> {
        WindowManager::preferred_window_size(self, id)
    }

    fn set_window_glow(
        &mut self,
        id: crate::dwm::WindowHandle,
        glow: Option<gevrek_engine::contract::GlowStrength>,
    ) {
        WindowManager::set_window_glow(self, id, glow);
    }

    fn decorate_window(
        &mut self,
        id: crate::dwm::WindowHandle,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> Result<()> {
        WindowManager::decorate_window(self, id, look, is_focused)
    }

    fn decorate_window_at(
        &mut self,
        id: crate::dwm::WindowHandle,
        at: gevrek_engine::container::Rect,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> Result<()> {
        WindowManager::decorate_window_at(self, id, at, look, is_focused)
    }

    fn update_border_position_explicit(
        &mut self,
        id: crate::dwm::WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        WindowManager::update_border_position_explicit(self, id, x, y, width, height)
    }

    fn toggle_title_bar(&mut self, id: crate::dwm::WindowHandle) -> Result<bool> {
        WindowManager::toggle_title_bar(self, id)
    }

    fn sync_stackbars(
        &mut self,
        desired: &[gevrek_engine::contract::StackbarContainerState],
        config: &gevrek_model::settings::StackbarProfile,
    ) -> Result<()> {
        WindowManager::sync_stackbars(self, desired, config)
    }

    fn launch(&mut self, launch: &gevrek_model::settings::LaunchProfile) -> Result<u32> {
        crate::host::process_manager::spawn_launch(launch).map(|child| child.id())
    }

    fn is_hidden_by_backend(&self, id: crate::dwm::WindowHandle) -> bool {
        crate::dwm::hidden_journal::is_hidden(id)
    }

    fn window_for_native_handle(&self, raw: isize) -> crate::dwm::WindowHandle {
        crate::dwm::WindowHandle(crate::dwm::hwnd_map::id_for(crate::dwm::hwnd_map::hwnd_of(
            raw,
        )))
    }

    fn untrack_window(&mut self, id: crate::dwm::WindowHandle) {
        WindowManager::untrack_window(self, id)
    }

    fn restore_journaled_windows(&mut self) -> usize {
        WindowManager::restore_journaled_windows(self)
    }
}
