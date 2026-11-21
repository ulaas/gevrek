//! The workspaces page: what workspaces each display has.
//!
//! Every display has workspaces of its own. Most of them are written down
//! once, for every display, and each display has its own of each; a
//! display can call one something else, lay it out another way, not have
//! it, or have one more that the others do not. The page shows that as it
//! is: the workspaces of every display, or those of one display, in their
//! order, with what the window manager has of them at this moment (which
//! is shown, how many windows it holds, what was opened for the moment
//! and is not written down).
//!
//! A workspace has a name, which is what keys and rules know it by, and
//! can have a title, which is what is shown of it. Renaming gives it a
//! title: nothing that names the workspace has to follow.
//!
//! What is done here is done to the configuration through the model's own
//! operations (`DesktopConfig::title_workspace_on` and the others), the
//! same the panel uses, so that the two cannot come to mean different
//! things by the same words.

use crate::document::Document;
use crate::form::Form;
use egui::{RichText, Ui};
use gevrek_model::layout::{LayoutType, WindowSizes};
use gevrek_model::settings::{
    DesktopConfig, DisplayRef, MonitorSelector, Titled,
};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;
use serde_json::Value;

/// A display the page can show the workspaces of.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub place: DisplayRef,
    /// What a person knows it by.
    pub label: String,
    /// The window manager has it now.
    pub attached: bool,
}

/// A workspace as the window manager has it now.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub name: String,
    pub label: String,
    pub windows: usize,
    /// It is the one its display shows.
    pub shown: bool,
    pub configured: bool,
}

/// The displays to choose among: those the window manager has, then
/// those the configuration names that are not attached now.
pub fn displays(desktop: &DesktopConfig, state: Option<&Value>) -> Vec<Shown> {
    let mut found: Vec<Shown> = state
        .and_then(|state| state["monitors"].as_array())
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, monitor)| {
            let place = DisplayRef::of_state(monitor, Some(index));
            let label = monitor["name"]
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(place.name())
                .to_string();
            Shown {
                place,
                label,
                attached: true,
            }
        })
        .filter(|shown| !shown.place.name().is_empty())
        .collect();
    for profile in &desktop.workspaces {
        let MonitorSelector::Name(name) = &profile.monitor else {
            continue;
        };
        if name.trim().is_empty() || found.iter().any(|shown| shown.place.is_called(name)) {
            continue;
        }
        found.push(Shown {
            place: DisplayRef::named(name),
            label: name.trim().to_string(),
            attached: false,
        });
    }
    found
}

/// The workspaces the window manager has on a display now.
pub fn live(state: Option<&Value>, place: &DisplayRef) -> Vec<Live> {
    let Some(monitor) = state
        .and_then(|state| state["monitors"].as_array())
        .into_iter()
        .flatten()
        .find(|monitor| DisplayRef::of_state(monitor, None).name() == place.name())
    else {
        return Vec::new();
    };
    let shown = &monitor["workspace"]["id"];
    monitor["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|workspace| {
            let name = workspace["name"].as_str().unwrap_or("").trim().to_string();
            let label = workspace["title"]
                .as_str()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or(&name)
                .to_string();
            let listed = workspace["detailed_windows"].as_array().map_or(0, Vec::len);
            let counted = workspace["total_windows"].as_u64().unwrap_or(0) as usize;
            Live {
                label,
                windows: listed.max(counted),
                shown: &workspace["id"] == shown,
                configured: workspace["configured"].as_bool().unwrap_or(false),
                name,
            }
        })
        .collect()
}

/// One workspace as a line of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub name: String,
    pub label: String,
    pub layout: LayoutType,
    /// How big its windows are, where the layout lets them be sized.
    pub sizes: WindowSizes,
    /// The master layouts' share and count (Dwindle: its first split).
    pub master_ratio: f32,
    pub master_count: usize,
    /// It stays when it is left empty.
    pub persistent: bool,
    /// The display starts with it.
    pub starts: bool,
    /// The display has an entry of its own for it.
    pub own: bool,
    /// No other display has it.
    pub only_here: bool,
    /// It is switched off here.
    pub off: bool,
    /// Where it is written down (`/desktop/workspaces/3`): the entry that
    /// counts here.
    pub pointer: String,
    /// What the window manager has of it now.
    pub live: Option<Live>,
}

fn same(one: &str, other: &str) -> bool {
    one.trim().eq_ignore_ascii_case(other.trim())
}

/// The workspaces of a display (`None`: of every display), in the order
/// they are written in, those that are switched off there too.
pub fn lines(desktop: &DesktopConfig, display: Option<&DisplayRef>, live: &[Live]) -> Vec<Line> {
    let mut names: Vec<&str> = Vec::new();
    for profile in &desktop.workspaces {
        let meant = display.is_none_or(|display| display.is_meant_by(&profile.monitor));
        if meant && !profile.name.trim().is_empty() && !names.iter().any(|n| same(n, &profile.name))
        {
            names.push(&profile.name);
        }
    }
    names
        .into_iter()
        .filter_map(|name| {
            // The entry that counts: for a display the one that names it
            // most closely, for every display the one that names fewest.
            let entries = desktop
                .workspaces
                .iter()
                .enumerate()
                .filter(|(_, profile)| same(&profile.name, name))
                .filter(|(_, profile)| {
                    display.is_none_or(|display| display.is_meant_by(&profile.monitor))
                });
            let (index, profile) = match display {
                Some(_) => entries.max_by_key(|(_, profile)| profile.monitor.closeness())?,
                None => entries.min_by_key(|(_, profile)| profile.monitor.closeness())?,
            };
            let own = display.is_some_and(|display| {
                matches!(&profile.monitor, MonitorSelector::Name(named) if display.is_called(named))
            });
            let elsewhere = desktop.workspaces.iter().enumerate().any(|(other, entry)| {
                other != index
                    && same(&entry.name, name)
                    && display.is_none_or(|display| display.is_meant_by(&entry.monitor))
            });
            Some(Line {
                name: profile.name.trim().to_string(),
                label: profile.label().to_string(),
                layout: profile.layout,
                sizes: profile.sizes.clone(),
                master_ratio: profile.master_ratio,
                master_count: profile.master_count,
                persistent: profile.persistent,
                starts: profile.default,
                own,
                only_here: own && !elsewhere,
                off: !profile.enabled,
                pointer: format!("/desktop/workspaces/{index}"),
                live: live.iter().find(|live| same(&live.name, name)).cloned(),
            })
        })
        .collect()
}

/// The workspaces a display has now that are not written down for it:
/// opened for the moment, gone when they are left empty.
pub fn of_the_moment(desktop: &DesktopConfig, display: &DisplayRef, live: &[Live]) -> Vec<Live> {
    live.iter()
        .filter(|live| {
            !live.name.is_empty() && desktop.workspace_on(display, &live.name).is_none()
        })
        .cloned()
        .collect()
}

/// Something asked of the page.
#[derive(Debug, Clone, PartialEq)]
pub enum Do {
    /// Shown by another title.
    Title(String, String),
    Layout(String, LayoutType),
    /// Its windows are sized so.
    Sizes(String, WindowSizes),
    /// The master area's share, and how many windows are in it.
    Master(String, f32, usize),
    /// It stays when it is left empty, or not.
    Persistent(String, bool),
    /// The display starts with it.
    Start(String),
    /// Taken out: on every display, or (a display's view) not had there.
    Remove(String),
    /// Had here again.
    Back(String),
    /// As every display has it.
    Share(String),
    /// Put before another (`None`: last).
    Move(String, Option<String>),
    Add,
    /// A workspace of the moment is written down, with what it is shown by.
    Keep(String, String),
    /// Known by another name to keys and rules, which follow.
    Name(String, String),
}

/// Carry out what was asked, on a display's workspaces or (`None`) on
/// every display's. An answer in words where it cannot be done.
pub fn carry_out(
    desktop: &mut DesktopConfig,
    display: Option<&DisplayRef>,
    what: Do,
) -> Result<(), String> {
    let titled = |titled: Titled, title: &str| match titled {
        Titled::Changed | Titled::Same => Ok(()),
        Titled::Taken => Err(format!(
            "Another workspace is called \u{201c}{}\u{201d} already.",
            title.trim()
        )),
        Titled::Unknown => Err("There is no such workspace any more.".to_string()),
    };
    match (what, display) {
        (Do::Title(name, title), Some(display)) => {
            titled(desktop.title_workspace_on(display, &name, &title), &title)
        }
        (Do::Title(name, title), None) => titled(desktop.title_workspace(&name, &title), &title),
        (Do::Layout(name, layout), Some(display)) => {
            desktop.edit_workspace_on(display, &name, |profile| profile.layout = layout);
            Ok(())
        }
        (Do::Layout(name, layout), None) => {
            desktop.edit_workspace(&name, |profile| profile.layout = layout);
            Ok(())
        }
        (Do::Sizes(name, sizes), Some(display)) => {
            desktop.edit_workspace_on(display, &name, |profile| profile.sizes = sizes.clone());
            Ok(())
        }
        (Do::Sizes(name, sizes), None) => {
            desktop.edit_workspace(&name, |profile| profile.sizes = sizes.clone());
            Ok(())
        }
        (Do::Master(name, ratio, count), Some(display)) => {
            desktop.edit_workspace_on(display, &name, |profile| {
                profile.master_ratio = ratio;
                profile.master_count = count;
            });
            Ok(())
        }
        (Do::Master(name, ratio, count), None) => {
            desktop.edit_workspace(&name, |profile| {
                profile.master_ratio = ratio;
                profile.master_count = count;
            });
            Ok(())
        }
        (Do::Persistent(name, keep), Some(display)) => {
            desktop.edit_workspace_on(display, &name, |profile| profile.persistent = keep);
            Ok(())
        }
        (Do::Persistent(name, keep), None) => {
            desktop.edit_workspace(&name, |profile| profile.persistent = keep);
            Ok(())
        }
        (Do::Start(name), display) => {
            desktop.start_with_workspace(display, &name);
            Ok(())
        }
        (Do::Remove(name), Some(display)) => {
            desktop.have_workspace_on(display, &name, false);
            Ok(())
        }
        (Do::Remove(name), None) => {
            desktop.remove_workspace(&name);
            Ok(())
        }
        (Do::Back(name), Some(display)) => {
            desktop.have_workspace_on(display, &name, true);
            Ok(())
        }
        (Do::Back(name), None) => {
            desktop.edit_workspace(&name, |profile| profile.enabled = true);
            Ok(())
        }
        (Do::Share(name), Some(display)) => {
            desktop.share_workspace_on(display, &name);
            Ok(())
        }
        (Do::Share(_), None) => Ok(()),
        (Do::Move(name, before), _) => {
            desktop.move_workspace(&name, before.as_deref());
            Ok(())
        }
        (Do::Add, display) => {
            desktop.add_workspace(display);
            Ok(())
        }
        (Do::Keep(name, title), Some(display)) => {
            if desktop.keep_workspace_on(display, &name, &title) {
                Ok(())
            } else {
                Err("This workspace is written down already.".to_string())
            }
        }
        (Do::Keep(..), None) => Ok(()),
        (Do::Name(old, new), _) => {
            if new.trim().is_empty() || same(&old, &new) && old.trim() == new.trim() {
                return Ok(());
            }
            if desktop.rename_workspace(&old, &new) {
                Ok(())
            } else {
                Err(format!(
                    "\u{201c}{}\u{201d} is another workspace's name already.",
                    new.trim()
                ))
            }
        }
    }
}

/// The page, with what it remembers between frames.
#[derive(Default)]
pub struct Page {
    /// The display whose workspaces are shown, by its name; none: those
    /// of every display.
    display: Option<String>,
    /// The workspace whose other settings are open.
    open: Option<String>,
    /// A text that is being typed and not yet kept: which field, of which
    /// workspace, and the text.
    typing: Option<(&'static str, String, String)>,
    /// Why the last thing asked could not be done.
    problem: Option<String>,
}

impl Page {
    /// A text field that keeps what is typed to itself until it is left
    /// (or Enter is pressed), then gives it back. So a name is not tried,
    /// and refused, letter by letter.
    fn text(
        &mut self,
        ui: &mut Ui,
        field: &'static str,
        name: &str,
        current: &str,
        width: f32,
    ) -> Option<String> {
        let typing = self
            .typing
            .as_ref()
            .filter(|(which, of, _)| *which == field && of == name)
            .map(|(_, _, text)| text.clone());
        let mut text = typing.clone().unwrap_or_else(|| current.to_string());
        let response = widgets::text_field(ui, &mut text, width);
        if response.changed() || (response.has_focus() && typing.is_none()) {
            self.typing = Some((field, name.to_string(), text.clone()));
        }
        if response.lost_focus() && typing.is_some() {
            self.typing = None;
            return (text.trim() != current.trim()).then_some(text);
        }
        None
    }

    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, state: Option<&Value>) {
        let Some(config) = document.config().cloned() else {
            ui.label("The configuration cannot be read as it stands: see Problems.");
            return;
        };
        let palette = theme::pal(ui.ctx());
        let desktop = &config.desktop;
        let known = displays(desktop, state);
        // (A display that is gone from both is not one to show.)
        if self
            .display
            .as_ref()
            .is_some_and(|name| !known.iter().any(|shown| shown.place.name() == name))
        {
            self.display = None;
        }
        let chosen = self
            .display
            .as_ref()
            .and_then(|name| known.iter().find(|shown| shown.place.name() == name))
            .cloned();
        let mut todo: Vec<Do> = Vec::new();

        // Whose workspaces.
        let labels: Vec<String> = known
            .iter()
            .map(|shown| {
                let label = widgets::ellipsize(&shown.label, 190.0);
                if shown.attached {
                    label
                } else {
                    format!("{label} (not attached)")
                }
            })
            .collect();
        let mut options: Vec<(Option<String>, &str)> = vec![(None, "Every display")];
        for (shown, label) in known.iter().zip(&labels) {
            options.push((Some(shown.place.name().to_string()), label.as_str()));
        }
        ui.horizontal_wrapped(|ui| {
            if let Some(picked) = widgets::segmented(
                ui,
                egui::Id::unique("the workspaces of which display"),
                Some(&self.display),
                &options,
                false,
            ) {
                self.display = picked;
                self.open = None;
                self.typing = None;
                self.problem = None;
            }
        });
        ui.add_space(4.0);
        let words = match &chosen {
            None => "Every display has its own of each of these. What is changed here is changed on every display.".to_string(),
            Some(shown) => format!(
                "The workspaces of {}. What is changed here is changed for this display alone.",
                shown.label
            ),
        };
        ui.label(RichText::new(words).color(palette.muted));
        if state.is_none() {
            ui.label(
                RichText::new(
                    "The window manager is not running: the displays it has, and what is open on them, are not shown.",
                )
                .size(12.0)
                .color(palette.faint),
            );
        }
        if let Some(problem) = self.problem.clone() {
            ui.add_space(4.0);
            widgets::banner(ui, palette.warn, icons::WARNING, |ui| {
                ui.label(problem);
            });
        }
        ui.add_space(8.0);

        let place = chosen.as_ref().map(|shown| &shown.place);
        let now = place.map(|place| live(state, place)).unwrap_or_default();
        let all = lines(desktop, place, &now);
        let layouts: Vec<(LayoutType, String, String)> = LayoutType::all()
            .into_iter()
            .map(|layout| (layout, layout.display_name().to_string(), String::new()))
            .collect();
        let there: Vec<&Line> = all.iter().filter(|line| !line.off).collect();
        if there.is_empty() {
            ui.label(RichText::new("No workspace is written down here yet.").color(palette.muted));
        }
        for (at, line) in there.iter().enumerate() {
            let frame = egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 4.0).max(200.0));
                    ui.horizontal(|ui| {
                        // Taken up by its handle, put down on another line.
                        let id = egui::Id::unique(("a workspace to put in order", &line.name));
                        ui.dnd_drag_source(id, line.name.clone(), |ui| {
                            ui.label(RichText::new(icons::DOTS_SIX_VERTICAL).color(palette.faint));
                        })
                        .response
                        .on_hover_text("Drag to put it in another place");
                        ui.label(
                            RichText::new(format!("{}.", at + 1))
                                .monospace()
                                .color(palette.faint),
                        );
                        if let Some(title) = self.text(ui, "title", &line.name, &line.label, 150.0)
                        {
                            todo.push(Do::Title(line.name.clone(), title));
                        }
                        if let Some(layout) = widgets::dropdown(
                            ui,
                            &format!("the layout of workspace {}", line.name),
                            &line.layout,
                            &layouts,
                            190.0,
                        ) {
                            todo.push(Do::Layout(line.name.clone(), layout));
                        }
                        let mut keep = line.persistent || line.starts;
                        ui.add_enabled_ui(!line.starts, |ui| {
                            if widgets::checkbox(ui, &mut keep, "Keep when empty")
                                .on_hover_text(
                                    "Off: it is opened when it is first asked for, and goes again when it is left empty.",
                                )
                                .changed()
                            {
                                todo.push(Do::Persistent(line.name.clone(), keep));
                            }
                        });
                        let mut starts = line.starts;
                        if ui
                            .radio(starts, "Start here")
                            .on_hover_text("The workspace a display shows when gevrek starts.")
                            .clicked()
                            && !starts
                        {
                            starts = true;
                            todo.push(Do::Start(line.name.clone()));
                        }
                        let _ = starts;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let tip = match (&chosen, line.only_here) {
                                (None, _) => "Take it out, on every display",
                                (Some(_), true) => "Take it out: only this display has it",
                                (Some(_), false) => "This display does not have it",
                            };
                            if widgets::small_icon_button(ui, icons::TRASH, tip).clicked() {
                                todo.push(Do::Remove(line.name.clone()));
                            }
                            let is_open = self.open.as_deref() == Some(line.name.as_str());
                            let caret = if is_open {
                                icons::CARET_UP
                            } else {
                                icons::CARET_DOWN
                            };
                            if widgets::small_icon_button(ui, caret, "Its other settings").clicked()
                            {
                                self.open = (!is_open).then(|| line.name.clone());
                            }
                            if line.own
                                && !line.only_here
                                && widgets::small_icon_button(
                                    ui,
                                    icons::ARROW_COUNTER_CLOCKWISE,
                                    "As every display has it",
                                )
                                .clicked()
                            {
                                todo.push(Do::Share(line.name.clone()));
                            }
                        });
                    });
                    // How big its windows are, as its layout lets them be.
                    if let Some(what) = sizes_row(ui, line) {
                        todo.push(what);
                    }
                    // What is to be said of it beside its settings, on a
                    // line of its own: nothing of it lies over a control.
                    let renamed = !same(&line.label, &line.name);
                    if renamed || line.own || line.live.is_some() {
                        ui.horizontal(|ui| {
                            ui.add_space(46.0);
                            if renamed {
                                ui.label(
                                    RichText::new(format!("name: {}", line.name))
                                        .size(11.5)
                                        .color(palette.faint),
                                )
                                .on_hover_text(
                                    "What keys and rules know it by. It stays when the workspace is renamed.",
                                );
                            }
                            if line.only_here {
                                widgets::pill(ui, "", "only this display has it", palette.accent);
                            } else if line.own {
                                widgets::pill(ui, "", "its own here", palette.accent);
                            }
                            if let Some(live) = &line.live {
                                if live.shown {
                                    widgets::pill(ui, "", "shown now", palette.ok);
                                }
                                let words = match live.windows {
                                    0 => "empty".to_string(),
                                    1 => "1 window".to_string(),
                                    count => format!("{count} windows"),
                                };
                                widgets::pill(ui, "", &words, palette.muted);
                            }
                        });
                    }
                    if self.open.as_deref() == Some(line.name.as_str()) {
                        ui.add_space(6.0);
                        ui.separator();
                        if chosen.is_some() && !line.own {
                            ui.label(
                                RichText::new(
                                    "These are every display's settings for it: changed here, they change everywhere.",
                                )
                                .size(12.0)
                                .color(palette.muted),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.label("Name for keys and rules");
                            if let Some(name) =
                                self.text(ui, "name", &line.name, &line.name, 150.0)
                            {
                                todo.push(Do::Name(line.name.clone(), name));
                            }
                            ui.label(
                                RichText::new("The keys that name it follow.")
                                    .size(12.0)
                                    .color(palette.faint),
                            );
                        });
                        let mut form = Form::new(document);
                        form.fields(
                            ui,
                            &line.pointer,
                            &[
                                "name",
                                "title",
                                "monitor",
                                "enabled",
                                "default",
                                "persistent",
                                "layout",
                                // (Set on the line of sizes above.)
                                "sizes",
                                "master_ratio",
                                "master_count",
                            ],
                        );
                    }
                })
                .response;
            // Put down on a line: before it, or behind it when it is let
            // go over its lower half.
            let behind = ui
                .ctx()
                .pointer_interact_pos()
                .is_some_and(|pointer| pointer.y > frame.rect.center().y);
            if let Some(name) = frame.dnd_release_payload::<String>() {
                let before = if behind {
                    there.get(at + 1).map(|next| next.name.clone())
                } else {
                    Some(line.name.clone())
                };
                todo.push(Do::Move((*name).clone(), before));
            }
            if frame.dnd_hover_payload::<String>().is_some() {
                let y = if behind {
                    frame.rect.bottom() + 1.0
                } else {
                    frame.rect.top() - 1.0
                };
                ui.painter().hline(
                    frame.rect.x_range(),
                    y,
                    egui::Stroke::new(2.0, palette.accent),
                );
            }
            ui.add_space(3.0);
        }
        ui.add_space(4.0);
        let add = match &chosen {
            None => "Add a workspace".to_string(),
            Some(shown) => format!("Add a workspace only {} has", shown.label),
        };
        if widgets::text_button(ui, icons::PLUS, &add).clicked() {
            todo.push(Do::Add);
        }

        // What this display does not have of what the others have.
        let off: Vec<&Line> = all.iter().filter(|line| line.off).collect();
        if !off.is_empty() {
            ui.add_space(10.0);
            widgets::caption(
                ui,
                if chosen.is_some() {
                    "Not on this display"
                } else {
                    "Switched off"
                },
            );
            for line in off {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&line.label).color(palette.muted));
                    if widgets::text_button(ui, icons::PLUS, "Have it again").clicked() {
                        todo.push(Do::Back(line.name.clone()));
                    }
                });
            }
        }

        // What is open on it now and not written down.
        if let Some(place) = place {
            let moment = of_the_moment(desktop, place, &now);
            if !moment.is_empty() {
                ui.add_space(10.0);
                widgets::caption(ui, "Open now, for the moment");
                ui.label(
                    RichText::new(
                        "Opened while gevrek ran. Each goes when it is left empty, unless it is kept.",
                    )
                    .size(12.0)
                    .color(palette.muted),
                );
                for live in moment {
                    ui.horizontal(|ui| {
                        ui.label(&live.label);
                        let words = match live.windows {
                            0 => "empty".to_string(),
                            1 => "1 window".to_string(),
                            count => format!("{count} windows"),
                        };
                        widgets::pill(ui, "", &words, palette.muted);
                        if live.shown {
                            widgets::pill(ui, "", "shown", palette.ok);
                        }
                        if widgets::text_button(ui, icons::PUSH_PIN, "Keep it").clicked() {
                            todo.push(Do::Keep(live.name.clone(), live.label.clone()));
                        }
                    });
                }
            }
        }

        if todo.is_empty() {
            return;
        }
        // (The form may have changed the document in this frame: what is
        // asked is done to the configuration as it is now.)
        let Some(mut changed) = document.config().cloned() else {
            return;
        };
        self.problem = None;
        // Sizes are dragged: the frames of one drag are one step to undo.
        let dragged = todo
            .iter()
            .all(|what| matches!(what, Do::Sizes(..) | Do::Master(..)));
        for what in todo {
            if let Do::Remove(name) | Do::Name(name, _) = &what {
                if self.open.as_deref() == Some(name.as_str()) {
                    self.open = None;
                }
            }
            if let Err(why) = carry_out(&mut changed.desktop, place, what) {
                self.problem = Some(why);
            }
        }
        if dragged {
            if let Ok(workspaces) = serde_json::to_value(&changed.desktop.workspaces) {
                document.set("/desktop/workspaces", workspaces);
                return;
            }
        }
        document.load(&changed);
    }
}

/// What can be sized under a layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizedBy {
    /// Scrolling: the width a column opens with, and by place.
    Strip,
    /// Columns (widths) or Rows (heights), by place.
    Places { across: bool },
    /// The master area's share, and (`count`) how many are in it.
    Master { count: bool },
    /// Dwindle: the share kept at the first split (`master_ratio`).
    FirstSplit,
    /// BSP and Spiral: the share kept at every split.
    Split,
}

impl SizedBy {
    pub fn of(layout: LayoutType) -> Option<SizedBy> {
        match layout {
            LayoutType::Scrolling => Some(SizedBy::Strip),
            LayoutType::Columns => Some(SizedBy::Places { across: true }),
            LayoutType::Rows => Some(SizedBy::Places { across: false }),
            LayoutType::MasterStack | LayoutType::CenteredMaster => {
                Some(SizedBy::Master { count: true })
            }
            LayoutType::UltrawideVerticalStack => Some(SizedBy::Master { count: false }),
            LayoutType::Dwindle => Some(SizedBy::FirstSplit),
            LayoutType::BSP | LayoutType::Spiral => Some(SizedBy::Split),
            LayoutType::Grid | LayoutType::Monocle | LayoutType::Floating => None,
        }
    }
}

/// A share as a whole percent, and back.
fn percent(share: f32) -> i32 {
    (share * 100.0).round() as i32
}

fn share(percent: i32) -> f32 {
    percent as f32 / 100.0
}

/// A share in percent, dragged or typed.
fn percent_box(
    ui: &mut Ui,
    value: &mut i32,
    range: std::ops::RangeInclusive<i32>,
    tip: &str,
) -> bool {
    ui.add(
        egui::DragValue::new(value)
            .range(range)
            .speed(0.5)
            .suffix("%"),
    )
    .on_hover_text(tip)
    .changed()
}

fn ordinal(place: usize) -> String {
    let number = place + 1;
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

/// The sizes of a workspace's windows, on a line of their own below its
/// settings, as its layout lets them be sized. What was changed, if
/// anything.
fn sizes_row(ui: &mut Ui, line: &Line) -> Option<Do> {
    let kind = SizedBy::of(line.layout)?;
    let palette = theme::pal(ui.ctx());
    let mut asked = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(46.0);
        ui.label(RichText::new(icons::ARROWS_OUT_LINE_HORIZONTAL).color(palette.faint));
        let mut sizes = line.sizes.clone();
        let (least, most) = (percent(WindowSizes::RANGE.0), percent(WindowSizes::RANGE.1));
        match kind {
            SizedBy::Strip | SizedBy::Places { .. } => {
                let mut changed = false;
                if kind == SizedBy::Strip {
                    ui.label("New columns");
                    let mut width = percent(sizes.column_width);
                    if percent_box(
                        ui,
                        &mut width,
                        least..=most,
                        "How wide a column is when its window opens, of the display's width. \
                         Wider than half: the next column stands cut off at the display's edge \
                         until the view moves to it.",
                    ) {
                        sizes.column_width = share(width);
                        changed = true;
                    }
                    ui.add_space(8.0);
                }
                let (words, tip) = match kind {
                    SizedBy::Places { across: false } => (
                        "Heights",
                        "Each row's height, of the display's, by its place. The rows after these share what is left.",
                    ),
                    SizedBy::Places { across: true } => (
                        "Widths",
                        "Each column's width, of the display's, by its place. The columns after these share what is left.",
                    ),
                    _ => (
                        "In view",
                        "The width of the column at this place in view, counted from the first column in view. As the view moves along the strip, each column takes the width of the place it comes to.",
                    ),
                };
                ui.label(words);
                let mut gone = None;
                for (place, value) in sizes.by_place.iter_mut().enumerate() {
                    ui.label(RichText::new(ordinal(place)).size(12.0).color(palette.faint));
                    let mut size = percent(*value);
                    if percent_box(ui, &mut size, least..=most, tip) {
                        *value = share(size);
                        changed = true;
                    }
                    if widgets::small_icon_button(ui, icons::X, "No size of its own").clicked() {
                        gone = Some(place);
                    }
                }
                if let Some(place) = gone {
                    sizes.by_place.remove(place);
                    changed = true;
                }
                if sizes.by_place.len() < WindowSizes::MOST_PLACES
                    && widgets::small_icon_button(
                        ui,
                        icons::PLUS,
                        &format!("Give the {} window a size of its own", ordinal(sizes.by_place.len())),
                    )
                    .clicked()
                {
                    let next = match kind {
                        SizedBy::Strip => sizes.column_width,
                        _ => 0.5,
                    };
                    sizes.by_place.push(next);
                    changed = true;
                }
                if sizes.by_place.is_empty() && kind != SizedBy::Strip {
                    ui.label(
                        RichText::new("every window as much as the others")
                            .size(12.0)
                            .color(palette.faint),
                    );
                }
                if changed {
                    asked = Some(Do::Sizes(line.name.clone(), sizes));
                }
            }
            SizedBy::Master { count } => {
                ui.label("Master area");
                let mut ratio = percent(line.master_ratio);
                let mut masters = line.master_count;
                let mut changed = percent_box(
                    ui,
                    &mut ratio,
                    10..=90,
                    "How much of the display the master area has.",
                );
                if count {
                    ui.add_space(8.0);
                    ui.label("Windows in it");
                    changed |= ui
                        .add(egui::DragValue::new(&mut masters).range(1..=8).speed(0.1))
                        .on_hover_text("How many windows share the master area.")
                        .changed();
                }
                if changed {
                    asked = Some(Do::Master(line.name.clone(), share(ratio), masters));
                }
            }
            SizedBy::FirstSplit => {
                ui.label("First window");
                let mut ratio = percent(line.master_ratio);
                if percent_box(
                    ui,
                    &mut ratio,
                    30..=70,
                    "The share the first window keeps when the second splits it; each window after keeps as much of what is left.",
                ) {
                    asked = Some(Do::Master(line.name.clone(), share(ratio), line.master_count));
                }
            }
            SizedBy::Split => {
                ui.label("At a split, a window keeps");
                let own = match line.layout {
                    LayoutType::Spiral => 0.618,
                    _ => 0.5,
                };
                let (split_least, split_most) = (
                    percent(WindowSizes::SPLIT_RANGE.0),
                    percent(WindowSizes::SPLIT_RANGE.1),
                );
                let mut kept = percent(sizes.split.unwrap_or(own));
                if percent_box(
                    ui,
                    &mut kept,
                    split_least..=split_most,
                    "The share of its room a window keeps when the next window splits it.",
                ) {
                    sizes.split = Some(share(kept));
                    asked = Some(Do::Sizes(line.name.clone(), sizes.clone()));
                }
                if sizes.split.is_some()
                    && widgets::small_icon_button(
                        ui,
                        icons::ARROW_COUNTER_CLOCKWISE,
                        "As the layout keeps it by itself",
                    )
                    .clicked()
                {
                    sizes.split = None;
                    asked = Some(Do::Sizes(line.name.clone(), sizes));
                }
            }
        }
    });
    asked
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        json!({"monitors": [
            {"name": "Left", "connector": "DP-1", "is_primary": true, "workspace": {"id": 2},
             "workspaces": [
                {"id": 1, "name": "1", "title": "Mail", "configured": true, "total_windows": 2},
                {"id": 2, "name": "2", "title": "2", "configured": true, "total_windows": 0},
                {"id": 9, "name": "12", "title": "12", "configured": false, "total_windows": 1}
             ]},
            {"name": "Right", "connector": "DP-2", "workspace": {"id": 5},
             "workspaces": [{"id": 5, "name": "1", "title": "1", "configured": true}]}
        ]})
    }

    #[test]
    fn the_displays_are_those_that_are_attached_and_those_that_are_named() {
        let mut desktop = gevrek_model::examples::starting_configuration().desktop;
        desktop.add_workspace(Some(&DisplayRef::named("HDMI-9")));
        desktop.add_workspace(Some(&DisplayRef::named("left")));
        let state = state();
        let known = displays(&desktop, Some(&state));
        let names: Vec<(&str, bool)> = known
            .iter()
            .map(|shown| (shown.label.as_str(), shown.attached))
            .collect();
        // (The one named `left` is the attached one, by its name.)
        assert_eq!(names, [("Left", true), ("Right", true), ("HDMI-9", false)]);
        assert_eq!(known[0].place.name(), "DP-1");
        // Nothing running: only what the configuration names.
        assert_eq!(displays(&desktop, None).len(), 2);
    }

    #[test]
    fn a_displays_lines_are_what_it_has_with_what_runs_of_it() {
        let mut desktop = gevrek_model::examples::starting_configuration().desktop;
        let left = DisplayRef::named("DP-1");
        assert!(desktop.title_workspace_on(&left, "1", "Mail").changed());
        desktop.have_workspace_on(&left, "3", false);
        let only = desktop.add_workspace(Some(&left));
        let state = state();
        let now = live(Some(&state), &left);
        assert_eq!(now.len(), 3);
        assert!(now[1].shown && !now[0].shown);

        let here = lines(&desktop, Some(&left), &now);
        let first = &here[0];
        assert_eq!((first.name.as_str(), first.label.as_str()), ("1", "Mail"));
        assert!(first.own && !first.only_here && !first.off);
        assert_eq!(first.live.as_ref().map(|live| live.windows), Some(2));
        assert!(!here[1].own);
        let third = here.iter().find(|line| line.name == "3").unwrap();
        assert!(third.off && third.own);
        let last = here.last().unwrap();
        assert_eq!(last.name, only);
        assert!(last.only_here);
        // Each line knows where it is written down.
        for line in &here {
            let index: usize = line.pointer.rsplit('/').next().unwrap().parse().unwrap();
            assert!(same(&desktop.workspaces[index].name, &line.name));
        }

        // Every display's: one line for each, by what all of them have.
        let every = lines(&desktop, None, &[]);
        assert_eq!(every[0].label, "1");
        assert!(every.iter().all(|line| !line.own && line.live.is_none()));
        assert!(every.iter().any(|line| line.name == only));
        assert!(!every.iter().find(|line| line.name == "3").unwrap().off);

        // What is open and not written down.
        let moment = of_the_moment(&desktop, &left, &now);
        assert_eq!(moment.len(), 1);
        assert_eq!(moment[0].name, "12");
    }

    #[test]
    fn what_is_asked_is_done_to_one_display_or_to_all() {
        let mut desktop = gevrek_model::examples::starting_configuration().desktop;
        let left = DisplayRef::named("DP-1");
        let right = DisplayRef::named("DP-2");
        let label = |desktop: &DesktopConfig, display: &DisplayRef, name: &str| {
            desktop
                .workspace_on(display, name)
                .map(|profile| profile.label().to_string())
        };
        let ask = |desktop: &mut DesktopConfig, display: Option<&DisplayRef>, what: Do| {
            carry_out(desktop, display, what)
        };
        let title = |name: &str, title: &str| Do::Title(name.to_string(), title.to_string());

        assert!(ask(&mut desktop, Some(&left), title("1", "Mail")).is_ok());
        assert_eq!(label(&desktop, &left, "1").as_deref(), Some("Mail"));
        assert_eq!(label(&desktop, &right, "1").as_deref(), Some("1"));
        assert!(ask(&mut desktop, Some(&left), title("2", "mail")).is_err());
        assert!(ask(&mut desktop, None, title("2", "Web")).is_ok());
        assert_eq!(label(&desktop, &left, "2").as_deref(), Some("Web"));
        assert_eq!(label(&desktop, &right, "2").as_deref(), Some("Web"));

        // A workspace of the moment kept, for its display.
        assert!(ask(&mut desktop, Some(&left), Do::Keep("12".into(), "Music".into())).is_ok());
        assert_eq!(label(&desktop, &left, "12").as_deref(), Some("Music"));
        assert_eq!(label(&desktop, &right, "12"), None);
        assert!(ask(&mut desktop, Some(&left), Do::Keep("12".into(), "x".into())).is_err());

        // Not on a display, and there again; taken out everywhere.
        assert!(ask(&mut desktop, Some(&left), Do::Remove("3".into())).is_ok());
        assert_eq!(label(&desktop, &left, "3"), None);
        assert!(label(&desktop, &right, "3").is_some());
        assert!(ask(&mut desktop, Some(&left), Do::Back("3".into())).is_ok());
        assert!(label(&desktop, &left, "3").is_some());
        assert!(ask(&mut desktop, None, Do::Remove("3".into())).is_ok());
        assert_eq!(label(&desktop, &right, "3"), None);

        // Another name for keys and rules: the keys follow.
        assert!(ask(&mut desktop, None, Do::Name("4".into(), "chat".into())).is_ok());
        assert!(label(&desktop, &left, "chat").is_some());
        assert!(ask(&mut desktop, None, Do::Name("chat".into(), "1".into())).is_err());
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
    }

    #[test]
    fn the_windows_sizes_are_set_on_one_display_or_on_all() {
        let mut desktop = gevrek_model::examples::starting_configuration().desktop;
        let left = DisplayRef::named("DP-1");
        let right = DisplayRef::named("DP-2");
        let wide = WindowSizes {
            column_width: 0.4,
            by_place: vec![0.7],
            split: None,
        };
        // One display's: the other keeps what every display has.
        assert!(carry_out(
            &mut desktop,
            Some(&left),
            Do::Sizes("1".into(), wide.clone())
        )
        .is_ok());
        assert_eq!(desktop.workspace_on(&left, "1").unwrap().sizes, wide);
        assert!(desktop
            .workspace_on(&right, "1")
            .unwrap()
            .sizes
            .is_default());
        // Every display's.
        assert!(carry_out(&mut desktop, None, Do::Master("2".into(), 0.65, 2)).is_ok());
        for display in [&left, &right] {
            let profile = desktop.workspace_on(display, "2").unwrap();
            assert_eq!((profile.master_ratio, profile.master_count), (0.65, 2));
        }
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
        // The line shows them.
        let shown = lines(&desktop, Some(&left), &[]);
        assert_eq!(
            shown.iter().find(|line| line.name == "1").unwrap().sizes,
            wide
        );
    }

    #[test]
    fn every_layout_but_those_with_nothing_to_size_has_sizes() {
        for layout in LayoutType::all() {
            let nothing = matches!(
                layout,
                LayoutType::Grid | LayoutType::Monocle | LayoutType::Floating
            );
            assert_eq!(SizedBy::of(layout).is_none(), nothing, "{layout:?}");
        }
        assert_eq!(ordinal(0), "1st");
        assert_eq!(ordinal(10), "11th");
        assert_eq!(ordinal(21), "22nd");
    }
}
