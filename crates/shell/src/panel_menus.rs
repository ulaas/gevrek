//! The menus a panel opens next to itself, the same on every system: the
//! layouts, the notices, the devices to play on, the windows a button of
//! the taskbar stands for, what can be done with one of them, the month
//! under the clock, and the panel's own menu.
//!
//! Each was written twice, once for the shell of each system, around the
//! same drawing. They are opened here, for whichever shell asks: what a
//! menu needs of the shell it is opened for (its look, a way to send the
//! window manager a command, the running configuration to change) is
//! [`PanelShell`], and each system's shell says how it does those. The
//! icons of the tray have a menu of their own on each system (the
//! programs' own on Linux, a list of the icons on Windows), and stay with
//! their shell.

use crate::panel::{self, Notice, PanelAsk, Task, TaskAction};
use gevrek_model::settings::PanelProfile;
use gevrek_shellkit::host::{Control, SurfaceId};
use gevrek_shellkit::Flow;
use gevrek_ui::theme;
use std::cell::RefCell;
use std::rc::Rc;

/// What a panel's menus need of the shell they are opened for.
pub trait PanelShell: 'static {
    /// How the shell's surfaces look.
    fn settings(&self) -> &gevrek_ui::Settings;
    fn accent(&self) -> egui::Color32;
    /// Send the window manager a command.
    fn dispatch(&self, command: String);
    /// The panels of the configuration.
    fn bars(&self) -> &[PanelProfile];
    /// The panel whose modules are being placed on the panel itself.
    fn arranging(&self) -> Option<&str>;
    fn set_arranging(&mut self, panel: Option<String>);
    /// Draw every panel again.
    fn redraw_panels(&self, control: &mut Control);
    /// Change the running configuration (`change` says whether it is
    /// another now) and have the window manager apply it.
    fn change_configuration(
        &mut self,
        change: &mut dyn FnMut(&mut gevrek_model::config::Config) -> bool,
    );
    /// A device to play on was chosen: `choose` makes it the one in use
    /// (it asks the system, and may take its moment).
    fn output_chosen(&self, choose: Box<dyn FnOnce() -> bool + Send>);
}

/// A menu of a panel, with what it shows.
pub enum Menu {
    /// The layouts, the one in use marked; `first`: a command to send
    /// before the layout is set (the workspace the display shows, where
    /// it has not the keyboard).
    Layouts {
        current: String,
        first: Option<String>,
    },
    Notices(Rc<Vec<Notice>>),
    /// The devices to play on (asked of the system as the menu opens).
    Sound,
    /// The windows a button of the taskbar stands for.
    Windows(Vec<Task>),
    /// What can be done with a window of the taskbar, and what each
    /// line sends.
    TaskActions(Task, [(TaskAction, Option<String>); 2]),
    /// The month under the clock.
    Calendar,
    /// The panel's own menu, for this panel.
    Panel(Box<PanelProfile>),
}

/// The look of the shell on a menu's surface: its fonts once, its colours
/// each frame. The colours, for what is drawn.
fn themed<S: PanelShell>(shell: &S, context: &egui::Context, fonts: &mut bool) -> theme::Palette {
    if !*fonts {
        theme::install_fonts_for(context, shell.settings(), gevrek_ui::fonts::Area::Panel);
        *fonts = true;
    }
    let palette = shell.settings().resolve(egui::Theme::Dark, shell.accent());
    theme::apply(context, &palette, shell.settings());
    palette
}

/// Open a menu of a panel next to `anchor` on the panel's surface. It
/// lives while `alive` is held by its drawing (the shell that opened it
/// keeps the other end, to know whether it is still there), and is put
/// away by a press elsewhere, Escape, or what is chosen in it. `None`:
/// there is nothing to show (no device to play on).
pub fn open<S: PanelShell>(
    shared: &Rc<RefCell<S>>,
    panel: SurfaceId,
    anchor: egui::Rect,
    menu: Menu,
    alive: Rc<()>,
    control: &mut Control,
) -> Option<SurfaceId> {
    let state = Rc::clone(shared);
    let mut fonts = false;
    // What every menu does first in a frame: its look, and Escape.
    macro_rules! begin {
        ($ui:expr, $shell:expr) => {{
            let context = $ui.ctx().clone();
            let palette = themed(&*$shell, &context, &mut fonts);
            // (Escape puts a menu away, as a press elsewhere does.)
            if panel::menu_escape(&context) {
                return Flow::Close;
            }
            palette
        }};
    }
    Some(match menu {
        Menu::Layouts { current, first } => {
            control.popup(panel, anchor, panel::layout_menu_size(), move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                let Some(layout) = panel::layout_menu(ui, &palette, &current) else {
                    return Flow::Continue;
                };
                if let Some(first) = &first {
                    shell.dispatch(first.clone());
                }
                shell.dispatch(panel::set_layout(layout));
                Flow::Close
            })
        }
        Menu::Notices(notices) => {
            let size = panel::notices_size(&notices);
            control.popup(panel, anchor, size, move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                panel::notices_list(ui, &palette, &notices);
                Flow::Continue
            })
        }
        Menu::Sound => {
            let outputs = gevrek_system::sound::outputs();
            if outputs.is_empty() {
                return None;
            }
            let size = panel::sound_menu_size(outputs.len());
            control.popup(panel, anchor, size, move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                let Some(id) = panel::sound_menu(ui, &palette, &outputs) else {
                    return Flow::Continue;
                };
                shell.output_chosen(Box::new(move || {
                    let chosen = gevrek_system::sound::choose_output(id);
                    if !chosen {
                        log::warn!("the device to play on was not changed");
                    }
                    chosen
                }));
                Flow::Close
            })
        }
        Menu::Windows(tasks) => {
            let size = panel::task_menu_size(tasks.len());
            control.popup(panel, anchor, size, move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                let Some(window) = panel::task_menu(ui, &palette, &tasks) else {
                    return Flow::Continue;
                };
                shell.dispatch(panel::go_to_window(window));
                Flow::Close
            })
        }
        Menu::TaskActions(task, commands) => {
            control.popup(panel, anchor, panel::task_actions_size(), move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                let Some(action) = panel::task_actions(ui, &palette, &task) else {
                    return Flow::Continue;
                };
                let command = commands
                    .iter()
                    .find(|(each, _)| *each == action)
                    .and_then(|(_, command)| command.clone());
                if let Some(command) = command {
                    shell.dispatch(command);
                }
                Flow::Close
            })
        }
        Menu::Calendar => {
            let today = jiff::Zoned::now().date();
            let mut month = (today.year(), today.month());
            control.popup(panel, anchor, panel::calendar_size(), move |ui, _| {
                let _held = &alive;
                let shell = state.borrow();
                let palette = begin!(ui, shell);
                // (Today as it is now: left open over midnight, it is right.)
                panel::calendar(ui, &palette, jiff::Zoned::now().date(), &mut month);
                Flow::Continue
            })
        }
        Menu::Panel(own) => {
            let count = shared.borrow().bars().len();
            let size = panel::panel_menu_size(&own, count);
            control.popup(panel, anchor, size, move |ui, control| {
                let _held = &alive;
                let ask = {
                    let shell = state.borrow();
                    let palette = begin!(ui, shell);
                    let arranging = shell.arranging() == Some(own.id.as_str());
                    panel::panel_menu(ui, &palette, &own, count, arranging)
                };
                let Some(ask) = ask else {
                    return Flow::Continue;
                };
                match ask {
                    PanelAsk::Settings => crate::control::open_studio("bar"),
                    // What stands on it is placed on the panel itself, or
                    // that is over: how the panel is drawn, for a while.
                    PanelAsk::Arrange(on) => {
                        let mut shell = state.borrow_mut();
                        shell.set_arranging(on.then(|| own.id.clone()));
                        shell.redraw_panels(control);
                    }
                    // Kept in the configuration: the window manager keeps
                    // the panels' room by it, and tells where each is.
                    _ => {
                        log::info!("the panel {:?} is asked: {ask:?}", own.id);
                        state.borrow_mut().change_configuration(&mut |config| {
                            panel::panel_asked(config, &own.id, ask)
                        });
                    }
                }
                Flow::Close
            })
        }
    })
}
