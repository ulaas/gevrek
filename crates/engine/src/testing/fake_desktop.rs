//! A desktop for tests of the core: windows are plain records, effects
//! change them, and the services a real backend renders do the minimum
//! (placement) or nothing. With it the executor and `WmCore` run in tests
//! without a real windowing system.

use crate::contract::{Desktop, Effect, StackbarContainerState, StateChange, WindowQueries};
use crate::outputs::MonitorHandle;
use crate::{Rect, WindowHandle};
use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use gevrek_model::settings::{RuleColor, ShadowPolicy};
use gevrek_model::window::WindowInfo;
use std::collections::BTreeMap;

/// One window on the fake desktop.
#[derive(Debug, Clone)]
pub struct FakeDesktopWindow {
    pub title: String,
    pub class: String,
    pub exe: String,
    pub rect: Rect,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub always_on_top: bool,
    pub opacity: u8,
    /// The WM hid it with an effect (as opposed to the application hiding
    /// its own window, which a test does by clearing `visible`).
    pub hidden_by_wm: bool,
    /// The application never gets narrower than this.
    pub min_width: Option<i32>,
}

#[derive(Debug)]
pub struct FakeDesktop {
    pub output: MonitorHandle,
    /// More outputs with their screen rectangles; a window is on the one
    /// that contains its centre, else on `output`.
    pub more_outputs: Vec<(MonitorHandle, Rect)>,
    /// How often the WM placed a window.
    pub placements: usize,
    pub windows: BTreeMap<WindowHandle, FakeDesktopWindow>,
    pub focused: Option<WindowHandle>,
    /// Every effect applied, in order.
    pub applied: Vec<Effect>,
    /// Programs asked to launch.
    pub launched: Vec<String>,
    next_window: isize,
}

impl FakeDesktop {
    /// A desktop with one output.
    pub fn new(output: MonitorHandle) -> Self {
        Self {
            output,
            more_outputs: Vec::new(),
            placements: 0,
            windows: BTreeMap::new(),
            focused: None,
            applied: Vec::new(),
            launched: Vec::new(),
            next_window: 100,
        }
    }

    /// Open a window; the caller then feeds the core `WindowEvent::Created`.
    pub fn open(&mut self, title: &str, exe: &str) -> WindowHandle {
        self.next_window += 1;
        let id = WindowHandle(self.next_window);
        self.windows.insert(
            id,
            FakeDesktopWindow {
                title: title.to_string(),
                class: format!("{exe}Class"),
                exe: exe.to_string(),
                rect: Rect::new(100, 100, 640, 480),
                visible: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                always_on_top: false,
                opacity: 255,
                hidden_by_wm: false,
                min_width: None,
            },
        );
        self.focused = Some(id);
        id
    }

    /// The window goes away; the caller then feeds `WindowEvent::Destroyed`.
    pub fn close(&mut self, id: WindowHandle) {
        self.windows.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
        }
    }

    pub fn exists_window(&self, id: WindowHandle) -> bool {
        self.windows.contains_key(&id)
    }

    pub fn window(&self, id: WindowHandle) -> &FakeDesktopWindow {
        &self.windows[&id]
    }

    fn get(&self, id: WindowHandle) -> Option<&FakeDesktopWindow> {
        self.windows.get(&id)
    }

    /// Open a window at a given place (on whichever output contains it).
    pub fn open_at(&mut self, title: &str, rect: Rect) -> WindowHandle {
        let id = self.open(title, "app.exe");
        self.windows.get_mut(&id).expect("just opened").rect = rect;
        id
    }

    /// Put the window where the WM asked, as far as the application allows.
    fn place(&mut self, id: WindowHandle, rect: Rect) {
        if let Some(window) = self.windows.get_mut(&id) {
            self.placements += 1;
            window.rect = rect;
            if let Some(min_width) = window.min_width {
                window.rect.width = window.rect.width.max(min_width);
            }
        }
    }
}

impl WindowQueries for FakeDesktop {
    fn active_window(&self) -> Option<WindowHandle> {
        self.focused
    }

    fn exists(&self, id: WindowHandle) -> bool {
        self.windows.contains_key(&id)
    }

    fn has_title(&self, id: WindowHandle, title: &str) -> Result<bool, String> {
        Ok(self
            .get(id)
            .is_some_and(|window| window.title.contains(title)))
    }

    fn position(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id).map(|window| (window.rect.x, window.rect.y))
    }

    fn size(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id)
            .map(|window| (window.rect.width, window.rect.height))
    }

    fn title(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.title.clone())
            .unwrap_or_default()
    }

    fn class(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.class.clone())
            .unwrap_or_default()
    }

    fn exe(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.exe.clone())
            .unwrap_or_default()
    }

    fn path(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| format!("C:\\Apps\\{}", window.exe))
            .unwrap_or_default()
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.windows
            .iter()
            .map(|(id, window)| WindowInfo {
                handle: *id,
                title: window.title.clone(),
                x: window.rect.x,
                y: window.rect.y,
                width: window.rect.width,
                height: window.rect.height,
                is_visible: window.visible,
                is_minimized: window.minimized,
                is_maximized: window.maximized,
            })
            .collect()
    }

    fn rect(&self, id: WindowHandle) -> Result<Rect, String> {
        self.get(id)
            .map(|window| window.rect)
            .ok_or_else(|| format!("no window {id:?}"))
    }

    fn is_minimized(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.minimized)
    }

    fn is_maximized(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.maximized)
    }

    fn is_fullscreen(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.fullscreen)
    }

    fn is_visible(&self, id: WindowHandle) -> bool {
        self.get(id)
            .is_some_and(|window| window.visible && !window.minimized)
    }

    fn is_always_on_top(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.always_on_top)
    }

    fn is_tracked(&self, id: WindowHandle) -> bool {
        self.exists(id)
    }

    fn pid(&self, id: WindowHandle) -> Option<u32> {
        self.exists(id).then_some(id.0 as u32)
    }

    fn initial_title(&self, id: WindowHandle) -> String {
        self.title(id)
    }

    fn initial_class(&self, id: WindowHandle) -> String {
        self.class(id)
    }

    fn hidden_by_app(&self, id: WindowHandle) -> bool {
        self.get(id)
            .is_some_and(|window| !window.visible && !window.hidden_by_wm)
    }

    fn is_decoration(&self, _id: WindowHandle) -> bool {
        false
    }

    fn has_border(&self, _id: WindowHandle) -> bool {
        false
    }

    fn opacity(&self, id: WindowHandle) -> u8 {
        self.get(id).map_or(255, |window| window.opacity)
    }

    fn border_color(&self, _id: WindowHandle) -> RuleColor {
        RuleColor::Default
    }

    fn caption_color(&self, _id: WindowHandle) -> RuleColor {
        RuleColor::Default
    }

    fn backdrop(&self, _id: WindowHandle) -> SystemBackdrop {
        SystemBackdrop::Auto
    }

    fn corners(&self, _id: WindowHandle) -> WindowCorner {
        WindowCorner::Default
    }

    fn shadow_policy(&self, _id: WindowHandle) -> ShadowPolicy {
        ShadowPolicy::Default
    }

    fn rule_facts(&self, id: WindowHandle) -> Option<crate::rules::WindowInfo> {
        let window = self.get(id)?;
        Some(crate::rules::WindowInfo {
            handle: id,
            title: window.title.clone(),
            class: window.class.clone(),
            exe: window.exe.clone(),
            path: self.path(id),
            app_id: crate::contract::app_id_of_exe(&window.exe),
            has_parent: false,
            is_visible: window.visible,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: Default::default(),
            workspace: None,
            workspace_index: None,
        })
    }

    fn ignored_by_rules(&self, _id: WindowHandle) -> bool {
        false
    }

    fn floats_by_rules(&self, _id: WindowHandle) -> bool {
        false
    }
}

impl Desktop for FakeDesktop {
    fn apply(&mut self, effect: &Effect) -> anyhow::Result<()> {
        self.applied.push(effect.clone());
        match effect {
            Effect::Configure { id, rect, .. } => self.place(*id, *rect),
            Effect::SetVisible { id, visible } => {
                if let Some(window) = self.windows.get_mut(id) {
                    window.visible = *visible;
                    window.hidden_by_wm = !*visible;
                }
            }
            Effect::Focus(target) => {
                self.focused = target.filter(|id| self.windows.contains_key(id));
            }
            Effect::Close(id) => self.close(*id),
            Effect::Opacity { id, alpha } => {
                if let Some(window) = self.windows.get_mut(id) {
                    window.opacity = *alpha;
                }
            }
            Effect::State { id, change } => {
                if let Some(window) = self.windows.get_mut(id) {
                    match change {
                        StateChange::Maximized(value) => window.maximized = *value,
                        StateChange::Minimized(value) => window.minimized = *value,
                        StateChange::Fullscreen(value) => window.fullscreen = *value,
                    }
                }
            }
            Effect::Stack {
                id,
                op: crate::contract::StackOp::AlwaysOnTop(value),
            } => {
                if let Some(window) = self.windows.get_mut(id) {
                    window.always_on_top = *value;
                }
            }
            // Recorded only.
            _ => {}
        }
        Ok(())
    }

    fn output_of(&self, id: WindowHandle) -> Option<MonitorHandle> {
        let rect = self.get(id)?.rect;
        let (x, y) = (rect.x + rect.width / 2, rect.y + rect.height / 2);
        Some(
            self.more_outputs
                .iter()
                .find(|(_, screen)| {
                    x >= screen.x
                        && x < screen.x + screen.width
                        && y >= screen.y
                        && y < screen.y + screen.height
                })
                .map_or(self.output, |(output, _)| *output),
        )
    }

    fn output_at_cursor(&self) -> Option<MonitorHandle> {
        Some(self.output)
    }

    fn primary_button_down(&self) -> bool {
        false
    }

    fn apply_animated_rectangles(
        &mut self,
        operations: &[(WindowHandle, Rect)],
        _scope: crate::animation_scheduler::TransitionScope,
    ) -> anyhow::Result<()> {
        for (id, rect) in operations {
            self.place(*id, *rect);
        }
        Ok(())
    }

    fn begin_workspace_transition(
        &mut self,
        _operations: &[(WindowHandle, Rect)],
        _offset_x: i32,
        _offset_y: i32,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn start_configured_window_enter(&mut self, _id: WindowHandle) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn start_window_animation(
        &mut self,
        _id: WindowHandle,
        _animation_type: crate::animations::AnimationType,
        _duration_ms: u32,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn is_animation_event_suppressed(&self, _id: WindowHandle) -> bool {
        false
    }

    fn preferred_window_size(&self, _id: WindowHandle) -> Option<(i32, i32)> {
        None
    }

    fn decorate_window(
        &mut self,
        _id: WindowHandle,
        _look: &crate::workspace::BorderLook,
        _is_focused: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn decorate_window_at(
        &mut self,
        _id: WindowHandle,
        _at: Rect,
        _look: &crate::workspace::BorderLook,
        _is_focused: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn update_border_position_explicit(
        &mut self,
        _id: WindowHandle,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn toggle_title_bar(&mut self, _id: WindowHandle) -> anyhow::Result<bool> {
        Ok(true)
    }

    fn sync_stackbars(
        &mut self,
        _desired: &[StackbarContainerState],
        _config: &gevrek_model::settings::StackbarProfile,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn launch(&mut self, launch: &gevrek_model::settings::LaunchProfile) -> anyhow::Result<u32> {
        self.launched.push(launch.program.clone());
        Ok(4242)
    }

    fn is_hidden_by_backend(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.hidden_by_wm)
    }

    fn window_for_native_handle(&self, raw: isize) -> WindowHandle {
        WindowHandle(raw)
    }

    fn untrack_window(&mut self, id: WindowHandle) {
        self.windows.remove(&id);
    }

    fn restore_journaled_windows(&mut self) -> usize {
        0
    }
}
