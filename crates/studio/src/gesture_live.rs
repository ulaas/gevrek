//! The fingers on the touchpad, as the window manager sees them now: on
//! the page of the mouse and the gestures, so that a gesture that is
//! being set up can be tried and watched — how many fingers it counts,
//! which way it takes them to go, and how far towards the distance a
//! gesture asks for.
//!
//! What is shown is the window manager's own account (the `gesture` part
//! of its state), asked for several times a second while the page is
//! open.

use egui::{RichText, Ui};
use gevrek_ui::theme;
use serde_json::Value;

/// Fingers that are moving, as the state has them.
#[derive(Debug, Clone, PartialEq)]
pub struct Moving {
    pub phase: String,
    pub fingers: u32,
    /// `left`, `right`, `up`, `down`, once it is decided.
    pub direction: Option<String>,
    /// How far towards what a gesture asks for, from 0 to 1 and beyond.
    pub progress: f32,
    pub delta: (f32, f32),
}

/// What the window manager says of gestures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    /// Gestures can be had at all (the backend sees a touchpad's fingers).
    pub available: bool,
    pub backend: String,
    pub devices: u32,
    /// How many gestures the configuration has.
    pub configured: u32,
    pub moving: Option<Moving>,
    pub error: Option<String>,
}

/// The gestures' part of a state.
pub fn reading(state: &Value) -> Reading {
    let said = &state["gesture"];
    let number = |value: &Value| value.as_f64().unwrap_or(0.0) as f32;
    Reading {
        available: said["available"].as_bool().unwrap_or(false),
        backend: said["backend"].as_str().unwrap_or_default().to_string(),
        devices: said["device_count"].as_u64().unwrap_or(0) as u32,
        configured: said["configured_count"].as_u64().unwrap_or(0) as u32,
        moving: said["active"].as_object().map(|active| Moving {
            phase: active
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            fingers: active.get("fingers").and_then(Value::as_u64).unwrap_or(0) as u32,
            direction: active
                .get("direction")
                .and_then(Value::as_str)
                .filter(|way| !way.is_empty())
                .map(str::to_string),
            progress: active.get("progress").map_or(0.0, number),
            delta: (
                active.get("delta_x").map_or(0.0, number),
                active.get("delta_y").map_or(0.0, number),
            ),
        }),
        error: said["last_error"]
            .as_str()
            .filter(|error| !error.is_empty())
            .map(str::to_string),
    }
}

/// Where the fingers are drawn on a pad of this size: so many dots in a
/// row about its middle, moved by what the fingers moved (a share of
/// the pad, kept on it).
pub fn dots(pad: egui::Rect, fingers: u32, delta: (f32, f32)) -> Vec<egui::Pos2> {
    let fingers = fingers.clamp(1, 5);
    let apart = (pad.width() / 9.0).min(26.0);
    let reach = egui::vec2(pad.width() * 0.32, pad.height() * 0.3);
    // (What the fingers moved is in shares of the touchpad, about -1 to 1.)
    let moved = egui::vec2(
        delta.0.clamp(-1.0, 1.0) * reach.x,
        delta.1.clamp(-1.0, 1.0) * reach.y,
    );
    let first = pad.center().x - apart * (fingers as f32 - 1.0) / 2.0;
    (0..fingers)
        .map(|finger| egui::pos2(first + apart * finger as f32, pad.center().y) + moved)
        .collect()
}

/// Draw what the window manager says of the fingers. `state`: its last
/// state, where it is connected.
pub fn show(ui: &mut Ui, state: Option<&Value>) {
    let palette = theme::pal(ui.ctx());
    let Some(state) = state else {
        ui.label(
            RichText::new("The fingers are shown here when a window manager is connected.")
                .color(palette.muted),
        );
        return;
    };
    let read = reading(state);
    ui.horizontal(|ui| {
        let (pad, _) = ui.allocate_exact_size(egui::vec2(260.0, 150.0), egui::Sense::hover());
        let painter = ui.painter_at(pad);
        painter.rect_filled(pad, 12.0, theme::mix(palette.surface, palette.text, 0.05));
        painter.rect_stroke(
            pad,
            12.0,
            egui::Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
        match &read.moving {
            Some(moving) => {
                // The way it is taken to go: a line from the middle.
                let way = match moving.direction.as_deref() {
                    Some("left") => Some(egui::vec2(-1.0, 0.0)),
                    Some("right") => Some(egui::vec2(1.0, 0.0)),
                    Some("up") => Some(egui::vec2(0.0, -1.0)),
                    Some("down") => Some(egui::vec2(0.0, 1.0)),
                    _ => None,
                };
                if let Some(way) = way {
                    let reach =
                        (pad.width().min(pad.height()) * 0.38) * moving.progress.clamp(0.0, 1.0);
                    painter.line_segment(
                        [pad.center(), pad.center() + way * reach],
                        egui::Stroke::new(3.0, palette.accent.gamma_multiply(0.6)),
                    );
                }
                let done = moving.progress >= 1.0;
                for dot in dots(pad, moving.fingers, moving.delta) {
                    painter.circle_filled(dot, 9.0, if done { palette.ok } else { palette.accent });
                }
            }
            None => {
                painter.text(
                    pad.center(),
                    egui::Align2::CENTER_CENTER,
                    "no fingers",
                    egui::FontId::proportional(13.0),
                    palette.faint,
                );
            }
        }
        ui.add_space(10.0);
        ui.vertical(|ui| {
            let line = |ui: &mut Ui, name: &str, value: String| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).color(palette.muted));
                    ui.label(RichText::new(value).color(palette.text));
                });
            };
            if read.available {
                line(
                    ui,
                    "Touchpads",
                    if read.backend.is_empty() {
                        read.devices.to_string()
                    } else {
                        format!("{} ({})", read.devices, read.backend)
                    },
                );
            } else {
                ui.label(
                    RichText::new("This window manager sees no touchpad's fingers.")
                        .color(palette.warn),
                );
            }
            line(ui, "Gestures set up", read.configured.to_string());
            match &read.moving {
                Some(moving) => {
                    line(ui, "Fingers", moving.fingers.to_string());
                    line(
                        ui,
                        "Going",
                        moving
                            .direction
                            .clone()
                            .unwrap_or_else(|| "not decided yet".to_string()),
                    );
                    line(
                        ui,
                        "Of the distance",
                        format!("{:.0} %", moving.progress.max(0.0) * 100.0),
                    );
                    ui.add(
                        egui::ProgressBar::new(moving.progress.clamp(0.0, 1.0))
                            .desired_width(180.0),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("Put three or four fingers on the touchpad and move them.")
                            .color(palette.faint),
                    );
                }
            }
            if let Some(error) = &read.error {
                ui.label(RichText::new(error).color(palette.error));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingers_are_read_from_the_state() {
        let state = serde_json::json!({"gesture": {
            "available": true, "backend": "libinput", "device_count": 1, "configured_count": 3,
            "active": {"phase": "update", "fingers": 3, "direction": "left", "progress": 0.5,
                       "delta_x": -0.25, "delta_y": 0.0},
        }});
        let read = reading(&state);
        assert!(read.available && read.devices == 1 && read.configured == 3);
        let moving = read.moving.expect("fingers that move");
        assert_eq!(
            (moving.fingers, moving.direction.as_deref()),
            (3, Some("left"))
        );
        assert_eq!((moving.progress, moving.delta), (0.5, (-0.25, 0.0)));
        // No fingers; nothing said at all.
        let still = reading(&serde_json::json!({"gesture": {"available": true}}));
        assert!(still.available && still.moving.is_none() && still.error.is_none());
        assert_eq!(reading(&serde_json::json!({})), Reading::default());
    }

    #[test]
    fn the_fingers_are_drawn_about_the_middle_and_stay_on_the_pad() {
        let pad = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(260.0, 150.0));
        let still = dots(pad, 3, (0.0, 0.0));
        assert_eq!(still.len(), 3);
        assert_eq!(still[1], pad.center());
        assert!(still[0].x < still[1].x && still[1].x < still[2].x);
        // Moved to the left: every dot further left, none off the pad,
        // however far the fingers went.
        let far = dots(pad, 4, (-9.0, 9.0));
        assert_eq!(far.len(), 4);
        assert!(far.iter().all(|dot| pad.contains(*dot)), "{far:?}");
        assert!(far[0].x < dots(pad, 4, (0.0, 0.0))[0].x);
        // No fingers counted yet: one dot all the same; never more than five.
        assert_eq!(dots(pad, 0, (0.0, 0.0)).len(), 1);
        assert_eq!(dots(pad, 9, (0.0, 0.0)).len(), 5);
    }
}
