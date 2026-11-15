//! Controls for things a plain slider or a text field says badly: a time of
//! the day, a span of hours that may cross midnight, and a value along a
//! track that means something at certain places (a colour temperature, a
//! speed with its usual settings).
//!
//! Like [`crate::widgets`], they take their colours from [`theme::pal`] and
//! know nothing of what they are used for: the caller says what a place on
//! a track looks like and what its set points are called.

use egui::{Color32, Sense, Ui};

use crate::theme;

/// The minutes of a day.
pub const DAY: u32 = 24 * 60;

/// A time of the day as it is written: `07:05`.
pub fn clock(minutes: u32) -> String {
    let minutes = minutes % DAY;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A time of the day brought to the nearest step (five minutes, a quarter
/// of an hour), within the day.
pub fn snapped(minutes: f32, step: u32) -> u32 {
    let step = step.max(1);
    let stepped = (minutes / step as f32).round() as i64 * i64::from(step);
    stepped.rem_euclid(i64::from(DAY)) as u32
}

/// How long it is from one time of the day to another, going on through
/// midnight where the second is the earlier one: `22:00` to `07:00` is
/// nine hours. The same time twice is the whole day.
pub fn span(from: u32, to: u32) -> u32 {
    let length = (to % DAY + DAY - from % DAY) % DAY;
    if length == 0 {
        DAY
    } else {
        length
    }
}

/// A span said as a person says it: `9 h`, `45 min`, `7 h 30 min`.
pub fn span_words(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, rest) => format!("{rest} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, rest) => format!("{hours} h {rest} min"),
    }
}

/// A time of the day chosen by its hour and its minute, each a number of
/// its own that is dragged, turned with the wheel, stepped with the arrow
/// keys or typed: never a text that could be no time at all. The minute
/// goes in steps of five when dragged or turned; typed, it is any minute.
/// Going past the end of the hour, or of the day, goes round.
pub fn time_field(ui: &mut Ui, minutes: &mut u32) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let before = *minutes % DAY;
    let (mut hour, mut minute) = ((before / 60) as i32, (before % 60) as i32);
    let height = ui.spacing().interact_size.y;
    let inner = egui::Frame::new()
        .fill(p.control)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(6, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.set_height(height);
                // (The numbers stand in the frame: no boxes of their own.)
                let visuals = ui.visuals_mut();
                for state in [
                    &mut visuals.widgets.inactive,
                    &mut visuals.widgets.hovered,
                    &mut visuals.widgets.active,
                ] {
                    state.bg_stroke = egui::Stroke::NONE;
                    state.weak_bg_fill = Color32::TRANSPARENT;
                    state.bg_fill = Color32::TRANSPARENT;
                }
                let two = |value: f64, _: std::ops::RangeInclusive<usize>| format!("{value:02.0}");
                let hours = ui.add_sized(
                    [26.0, height],
                    egui::DragValue::new(&mut hour)
                        .speed(0.08)
                        .custom_formatter(two),
                );
                ui.label(egui::RichText::new(":").color(p.muted));
                let rest = ui.add_sized(
                    [26.0, height],
                    egui::DragValue::new(&mut minute)
                        .speed(0.25)
                        .custom_formatter(two),
                );
                // The wheel over a number turns it (and not the page).
                let turned = |response: &egui::Response| -> i32 {
                    if !response.hovered() {
                        return 0;
                    }
                    let notches = ui.input_mut(|input| {
                        // (Each notch of the wheel as it was turned, not
                        // the smoothed way the page would scroll by.)
                        let turned: f32 = input
                            .raw
                            .events
                            .iter()
                            .map(|event| match event {
                                egui::Event::MouseWheel { delta, .. } => delta.y.signum(),
                                _ => 0.0,
                            })
                            .sum();
                        input.smooth_scroll_delta = egui::Vec2::ZERO;
                        turned
                    });
                    notches as i32
                };
                hour += turned(&hours);
                minute += turned(&rest) * 5;
                hours.union(rest)
            })
            .inner
        });
    let mut response = inner.inner;
    // Past the end of the hour the hour goes on; past the day's, round.
    let total = i64::from(hour) * 60 + i64::from(minute);
    let now = total.rem_euclid(i64::from(DAY)) as u32;
    if now != before {
        *minutes = now;
        response.mark_changed();
    }
    response
}

/// What a [`day_band`] is drawn with.
pub struct DayBand {
    /// The colour of the span.
    pub fill: Color32,
    /// The time it is now, marked on the band.
    pub now: Option<u32>,
    /// The step the handles go in, in minutes.
    pub step: u32,
    /// The span is not in use (switched off): drawn faintly.
    pub dim: bool,
}

impl Default for DayBand {
    fn default() -> Self {
        Self {
            fill: Color32::from_rgb(240, 160, 60),
            now: None,
            step: 5,
            dim: false,
        }
    }
}

/// Where a time of the day is along a band that runs from noon to noon
/// (so that the night, which is what such a span is mostly for, lies whole
/// in its middle), from 0 to 1.
pub fn along_the_day(minutes: u32) -> f32 {
    ((minutes % DAY + DAY - DAY / 2) % DAY) as f32 / DAY as f32
}

/// The time of the day at a place along that band.
pub fn time_along(part: f32) -> f32 {
    (part.clamp(0.0, 1.0) * DAY as f32 + (DAY / 2) as f32) % DAY as f32
}

/// The parts of the band (from, to; each 0 to 1) that a span covers: one,
/// or two where it goes over the band's ends (a span that holds noon).
pub fn band_parts(from: u32, to: u32) -> Vec<(f32, f32)> {
    let (start, length) = (along_the_day(from), span(from, to) as f32 / DAY as f32);
    if length >= 1.0 {
        return vec![(0.0, 1.0)];
    }
    let end = start + length;
    if end <= 1.0 {
        vec![(start, end)]
    } else {
        vec![(start, 1.0), (0.0, end - 1.0)]
    }
}

/// A span of the day, seen whole and set by its two ends: a band of
/// twenty-four hours from noon to noon, the span filled, a handle at its
/// beginning and one at its end, each dragged along the band (a press on
/// the band brings the nearer one there). It may cross midnight, and may
/// hold noon. The hours are written under it; the time a handle is at is
/// written over it while it is held or the pointer rests on it.
pub fn day_band(ui: &mut Ui, from: &mut u32, to: &mut u32, look: &DayBand) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let knob = 9.0;
    let width = ui.available_width().clamp(260.0, 620.0);
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(width, 64.0), Sense::click_and_drag());
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left() + knob + 2.0, rect.top() + 20.0),
        egui::pos2(rect.right() - knob - 2.0, rect.top() + 34.0),
    );
    let x_of = |minutes: u32| egui::lerp(track.left()..=track.right(), along_the_day(minutes));
    let ends = [x_of(*from), x_of(*to)];

    // Which end the pointer has: the one it took when it came down, held
    // until it is let go.
    let held_id = response.id.with("held");
    let mut held: Option<usize> = ui.data(|data| data.get_temp(held_id));
    let pointer = response.interact_pointer_pos();
    if let (Some(at), true) = (pointer, response.drag_started() || response.clicked()) {
        // (Where both are at one place, the end is taken to the right and
        // the beginning to the left.)
        let nearer = if (at.x - ends[0]).abs() < (at.x - ends[1]).abs()
            || ((at.x - ends[0]).abs() == (at.x - ends[1]).abs() && at.x < ends[0])
        {
            0
        } else {
            1
        };
        held = Some(nearer);
    }
    if let (Some(which), Some(at)) = (held, pointer) {
        if response.dragged() || response.clicked() {
            let part = (at.x - track.left()) / track.width();
            let time = snapped(time_along(part), look.step);
            let end = if which == 0 { &mut *from } else { &mut *to };
            if *end != time {
                *end = time;
                response.mark_changed();
            }
        }
    }
    if !response.dragged() && !response.is_pointer_button_down_on() {
        held = None;
    }
    ui.data_mut(|data| match held {
        Some(which) => {
            data.insert_temp(held_id, which);
        }
        None => data.remove::<usize>(held_id),
    });
    // The arrow keys move the end that was last held (the beginning, at
    // first), a step at a time.
    if response.has_focus() {
        let (left, right) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::ArrowRight),
            )
        });
        if left || right {
            let by = if right { look.step as i64 } else { -(look.step as i64) };
            *from = (i64::from(*from) + by).rem_euclid(i64::from(DAY)) as u32;
            response.mark_changed();
        }
    }

    let painter = ui.painter();
    let fill = if look.dim {
        theme::mix(look.fill, p.control_active(), 0.65)
    } else {
        look.fill
    };
    painter.rect_filled(track, 7.0, p.control_active());
    for (start, end) in band_parts(*from, *to) {
        let piece = egui::Rect::from_min_max(
            egui::pos2(egui::lerp(track.left()..=track.right(), start), track.top()),
            egui::pos2(egui::lerp(track.left()..=track.right(), end), track.bottom()),
        );
        painter.rect_filled(piece, 7.0, fill);
    }
    // The hours: a mark every three, a number every six.
    for hour in (0..=24).step_by(3) {
        let x = egui::lerp(track.left()..=track.right(), hour as f32 / 24.0);
        let named = hour % 6 == 0;
        painter.line_segment(
            [
                egui::pos2(x, track.bottom() + 3.0),
                egui::pos2(x, track.bottom() + if named { 8.0 } else { 6.0 }),
            ],
            egui::Stroke::new(1.0, p.faint),
        );
        if named {
            painter.text(
                egui::pos2(x, track.bottom() + 10.0),
                egui::Align2::CENTER_TOP,
                format!("{:02}:00", (hour + 12) % 24),
                egui::FontId::proportional(11.0),
                p.faint,
            );
        }
    }
    // Now.
    if let Some(now) = look.now {
        let x = x_of(now);
        painter.line_segment(
            [
                egui::pos2(x, track.top() - 3.0),
                egui::pos2(x, track.bottom() + 3.0),
            ],
            egui::Stroke::new(1.5, p.text),
        );
    }
    // The two ends.
    let hover = response.hover_pos();
    for (which, minutes) in [(0usize, *from), (1usize, *to)] {
        let centre = egui::pos2(x_of(minutes), track.center().y);
        let near = hover.is_some_and(|at| (at.x - centre.x).abs() <= knob + 3.0);
        let hot = held == Some(which) || (held.is_none() && near);
        if hot {
            painter.circle_filled(centre, knob + 5.0, fill.gamma_multiply(0.22));
        }
        painter.circle_filled(
            centre + egui::vec2(0.0, 1.0),
            knob,
            Color32::from_black_alpha(60),
        );
        painter.circle(
            centre,
            knob,
            if p.dark { p.text } else { Color32::WHITE },
            egui::Stroke::new(2.0, fill),
        );
        if hot {
            painter.text(
                egui::pos2(centre.x, track.top() - 5.0),
                egui::Align2::CENTER_BOTTOM,
                clock(minutes),
                egui::FontId::proportional(12.0),
                p.text,
            );
        }
    }
    if response.has_focus() {
        painter.rect_stroke(
            track.expand(4.0),
            10.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Outside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::slider(
            ui.is_enabled(),
            f64::from(*from),
            format!("from {} to {}", clock(*from), clock(*to)),
        )
    });
    response
}

/// A place along a [`StopSlider`]'s track that has a name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub value: f32,
    pub name: &'static str,
}

/// The set point a value stands at, if it stands at one (within a
/// hundredth of the range).
pub fn stop_at(stops: &[Stop], value: f32, range: (f32, f32)) -> Option<usize> {
    let near = (range.1 - range.0).abs() / 100.0;
    stops
        .iter()
        .position(|stop| (stop.value - value).abs() <= near)
}

/// A value brought to the set point it is close to (within a fortieth of
/// the range): a knob that is let go near one lands on it.
pub fn caught(stops: &[Stop], value: f32, range: (f32, f32)) -> f32 {
    let reach = (range.1 - range.0).abs() / 40.0;
    stops
        .iter()
        .map(|stop| stop.value)
        .filter(|stop| (stop - value).abs() <= reach)
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
        .unwrap_or(value)
}

/// A slider whose track shows what its values are (painted by the caller:
/// a colour for every place along it), with set points that have names:
/// marked on the track, caught by the knob when it comes near, and listed
/// under it to be pressed. The value itself stands beside it and can be
/// typed.
pub struct StopSlider<'a> {
    value: &'a mut f32,
    range: (f32, f32),
    stops: &'a [Stop],
    paint: Option<&'a dyn Fn(f32) -> Color32>,
    suffix: &'a str,
    step: f32,
}

impl<'a> StopSlider<'a> {
    pub fn new(value: &'a mut f32, range: std::ops::RangeInclusive<f32>) -> Self {
        Self {
            value,
            range: (*range.start(), *range.end()),
            stops: &[],
            paint: None,
            suffix: "",
            step: 1.0,
        }
    }

    pub fn stops(mut self, stops: &'a [Stop]) -> Self {
        self.stops = stops;
        self
    }

    /// The colour the track has at a value (and the knob, at its own).
    pub fn painted(mut self, paint: &'a dyn Fn(f32) -> Color32) -> Self {
        self.paint = Some(paint);
        self
    }

    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }

    /// What the value is rounded to.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step.max(f32::EPSILON);
        self
    }
}

impl egui::Widget for StopSlider<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let p = theme::pal(ui.ctx());
        let (low, high) = self.range;
        let part_of = |value: f32| ((value - low) / (high - low).max(f32::EPSILON)).clamp(0.0, 1.0);
        let rounded = |value: f32| ((value / self.step).round() * self.step).clamp(low, high);
        let before = *self.value;
        let mut value = before;
        let knob = 9.0;

        let inner = ui.vertical(|ui| {
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let width = (ui.available_width() - 84.0).clamp(180.0, 420.0);
                let size = egui::vec2(width, ui.spacing().interact_size.y);
                let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let track = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(rect.width() - 2.0 * knob, 8.0),
                );
                if let Some(at) = response.interact_pointer_pos() {
                    if response.dragged() || response.clicked() {
                        let part = ((at.x - track.left()) / track.width()).clamp(0.0, 1.0);
                        let wanted = caught(
                            self.stops,
                            low + part * (high - low),
                            self.range,
                        );
                        value = rounded(wanted);
                    }
                }
                if response.has_focus() {
                    let (left, right) = ui.input(|input| {
                        (
                            input.key_pressed(egui::Key::ArrowLeft),
                            input.key_pressed(egui::Key::ArrowRight),
                        )
                    });
                    if left || right {
                        let by = (high - low) / 50.0;
                        value = rounded(value + if right { by } else { -by });
                    }
                }
                response.widget_info(|| {
                    egui::WidgetInfo::slider(ui.is_enabled(), f64::from(value), "")
                });

                let painter = ui.painter();
                let enabled = ui.is_enabled();
                match self.paint {
                    // The track in the colours of its values, piece by piece.
                    Some(paint) => {
                        let pieces = 48;
                        for piece in 0..pieces {
                            let (start, end) = (
                                piece as f32 / pieces as f32,
                                (piece + 1) as f32 / pieces as f32,
                            );
                            let colour = paint(low + (start + end) / 2.0 * (high - low));
                            let colour = if enabled {
                                colour
                            } else {
                                theme::mix(colour, p.control_active(), 0.6)
                            };
                            let part = egui::Rect::from_min_max(
                                egui::pos2(
                                    egui::lerp(track.left()..=track.right(), start),
                                    track.top(),
                                ),
                                egui::pos2(
                                    egui::lerp(track.left()..=track.right(), end) + 0.5,
                                    track.bottom(),
                                ),
                            );
                            let round = if piece == 0 || piece == pieces - 1 { 4.0 } else { 0.0 };
                            painter.rect_filled(part, round, colour);
                        }
                    }
                    None => {
                        painter.rect_filled(track, 4.0, p.control_active());
                        let filled = egui::Rect::from_min_max(
                            track.left_top(),
                            egui::pos2(
                                egui::lerp(track.left()..=track.right(), part_of(value)),
                                track.bottom(),
                            ),
                        );
                        painter.rect_filled(filled, 4.0, if enabled { p.accent } else { p.faint });
                    }
                }
                // The set points, marked on the track.
                for stop in self.stops {
                    // (Not at the track's very ends: there is its end.)
                    if stop.value <= low || stop.value >= high {
                        continue;
                    }
                    let x = egui::lerp(track.left()..=track.right(), part_of(stop.value));
                    painter.circle_filled(
                        egui::pos2(x, track.center().y),
                        2.0,
                        Color32::from_black_alpha(110),
                    );
                }
                let centre = egui::pos2(
                    egui::lerp(track.left()..=track.right(), part_of(value)),
                    rect.center().y,
                );
                let hot = ui.ctx().animate_bool_responsive(
                    response.id.with("hot"),
                    enabled && (response.hovered() || response.dragged() || response.has_focus()),
                );
                let ring = if enabled { p.accent } else { p.faint };
                if hot > 0.0 {
                    painter.circle_filled(centre, knob + 5.0 * hot, ring.gamma_multiply(0.18 * hot));
                }
                painter.circle_filled(
                    centre + egui::vec2(0.0, 1.0),
                    knob,
                    Color32::from_black_alpha(70),
                );
                let inside = match self.paint {
                    Some(paint) => paint(value),
                    None if p.dark => p.text,
                    None => Color32::WHITE,
                };
                painter.circle(
                    centre,
                    knob,
                    inside,
                    egui::Stroke::new(2.0, if p.dark { p.text } else { Color32::WHITE }),
                );
                painter.circle_stroke(centre, knob + 1.5, egui::Stroke::new(1.0, ring));

                let typed = ui.add_sized(
                    [70.0, size.y],
                    egui::DragValue::new(&mut value)
                        .speed((high - low) / 300.0)
                        .range(low..=high)
                        .fixed_decimals(if self.step >= 1.0 { 0 } else { 2 })
                        .suffix(self.suffix),
                );
                response = response.union(typed);
                response
            });
            let mut response = row.inner;
            // The set points by name: the one the value stands at is the
            // one that is chosen.
            if !self.stops.is_empty() {
                let chosen = stop_at(self.stops, value, self.range);
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for (index, stop) in self.stops.iter().enumerate() {
                        let is = chosen == Some(index);
                        let text = egui::RichText::new(stop.name)
                            .size(12.5)
                            .color(if is { p.text } else { p.muted });
                        let mut button = egui::Button::new(text)
                            .corner_radius(12)
                            .min_size(egui::vec2(0.0, 24.0));
                        if let Some(paint) = self.paint {
                            // (A dot of the colour it stands for.)
                            let dot = paint(stop.value);
                            button = egui::Button::new(
                                egui::RichText::new(format!("     {}", stop.name))
                                    .size(12.5)
                                    .color(if is { p.text } else { p.muted }),
                            )
                            .corner_radius(12)
                            .min_size(egui::vec2(0.0, 24.0));
                            let pressed = ui.add(button.selected(is));
                            ui.painter().circle_filled(
                                egui::pos2(pressed.rect.left() + 13.0, pressed.rect.center().y),
                                4.5,
                                dot,
                            );
                            if pressed.clicked() {
                                value = rounded(stop.value);
                            }
                            response = response.union(pressed);
                            continue;
                        }
                        let pressed = ui.add(button.selected(is));
                        if pressed.clicked() {
                            value = rounded(stop.value);
                        }
                        response = response.union(pressed);
                    }
                });
            }
            response
        });
        let mut response = inner.inner;
        if value != before {
            *self.value = value;
            response.mark_changed();
        }
        response
    }
}

/// Words on one line, cut with an ellipsis where they are wider than
/// their room.
pub fn one_line(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    colour: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(8.0));
    ui.painter().layout_job(job)
}

/// A tile for something that is on or off, on a surface with little
/// room: its sign in a disc, its name, and a line that says how it
/// stands. A press switches it. A tile that has more to it (a list of
/// what it found, a choice) has an arrow at its end, pressed by itself,
/// that opens that under the tiles.
pub struct Tile<'a> {
    pub icon: &'a str,
    pub name: &'a str,
    /// How it stands, in a few words.
    pub state: &'a str,
    pub on: bool,
    /// It has more to show; `Some(true)`: that is open.
    pub more: Option<bool>,
    /// The system is still following a press.
    pub busy: bool,
}

/// What was done to a [`Tile`].
pub struct TilePressed {
    pub response: egui::Response,
    /// The tile itself was pressed.
    pub switched: bool,
    /// Its arrow was pressed.
    pub more: bool,
}

pub fn tile(ui: &mut Ui, size: egui::Vec2, tile: &Tile<'_>) -> TilePressed {
    let p = theme::pal(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let arrow = tile.more.map(|_| {
        egui::Rect::from_min_max(egui::pos2(rect.right() - 30.0, rect.top()), rect.right_bottom())
    });
    let more = arrow.map(|arrow| ui.interact(arrow, response.id.with("more"), Sense::click()));
    let more_pressed = more.as_ref().is_some_and(egui::Response::clicked);
    let switched = response.clicked() && !more_pressed;

    let lit = ui.ctx().animate_bool_responsive(response.id.with("on"), tile.on);
    let hovered = response.hovered() || more.as_ref().is_some_and(egui::Response::hovered);
    let ground = theme::mix(
        if hovered { p.control_hover() } else { p.control },
        p.accent,
        0.24 * lit,
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        12.0,
        ground,
        egui::Stroke::new(1.0, theme::mix(p.border, p.accent, 0.55 * lit)),
        egui::StrokeKind::Inside,
    );
    // The sign, in a disc that is the accent's while it is on.
    let disc = egui::pos2(rect.left() + 26.0, rect.center().y);
    painter.circle_filled(disc, 15.0, theme::mix(p.control_active(), p.accent, lit));
    painter.text(
        disc,
        egui::Align2::CENTER_CENTER,
        tile.icon,
        egui::FontId::proportional(16.0),
        theme::mix(p.text, p.on_accent(), lit),
    );
    let left = rect.left() + 50.0;
    let room = rect.right() - left - if arrow.is_some() { 30.0 } else { 10.0 };
    let name = one_line(ui, tile.name, egui::FontId::proportional(13.5), p.text, room);
    let state = one_line(ui, tile.state, egui::FontId::proportional(11.5), p.muted, room);
    let tall = name.size().y + state.size().y + 1.0;
    let top = rect.center().y - tall / 2.0;
    painter.galley(egui::pos2(left, top), name.clone(), p.text);
    painter.galley(
        egui::pos2(left, top + name.size().y + 1.0),
        state,
        p.muted,
    );
    if let (Some(arrow), Some(open)) = (arrow, tile.more) {
        if tile.busy {
            ui.put(
                egui::Rect::from_center_size(arrow.center(), egui::vec2(14.0, 14.0)),
                egui::Spinner::new().size(14.0),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        } else {
            let hot = more.as_ref().is_some_and(egui::Response::hovered);
            ui.painter().text(
                arrow.center(),
                egui::Align2::CENTER_CENTER,
                if open {
                    theme::icons::CARET_DOWN
                } else {
                    theme::icons::CARET_RIGHT
                },
                egui::FontId::proportional(14.0),
                if hot { p.text } else { p.muted },
            );
        }
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            14.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Outside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), tile.on, tile.name)
    });
    TilePressed {
        response,
        switched,
        more: more_pressed,
    }
}

/// What was done to a [`level`].
pub struct LevelMoved {
    /// Its sign was pressed (mute, where the sign means that).
    pub sign: bool,
    /// The slider: `dragged` while it is held, `drag_stopped` when it is
    /// let go, `changed` when its value is another.
    pub slider: egui::Response,
}

/// A level from nothing to full on a surface with little room: its sign
/// (pressed by itself, where that means something: mute), a name where
/// several stand under each other, a slider as wide as the room that is
/// left, and the level in words where it is said.
pub fn level(
    ui: &mut Ui,
    icon: &str,
    tip: &str,
    name: Option<&str>,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    said: Option<&str>,
) -> LevelMoved {
    let p = theme::pal(ui.ctx());
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let height = ui.spacing().interact_size.y;
        let sign = ui
            .add(
                egui::Button::new(egui::RichText::new(icon).size(17.0))
                    .frame(false)
                    .min_size(egui::vec2(28.0, height)),
            )
            .on_hover_text(tip);
        if let Some(name) = name {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(92.0, height), Sense::hover());
            let words = one_line(ui, name, egui::FontId::proportional(13.0), p.muted, rect.width());
            ui.painter().galley(
                egui::pos2(rect.left(), rect.center().y - words.size().y / 2.0),
                words,
                p.muted,
            );
        }
        let words = if said.is_some() { 46.0 } else { 0.0 };
        let width = (ui.available_width() - words).max(60.0);
        let (rect, mut slider) =
            ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
        let knob = 8.0;
        let track = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(rect.width() - 2.0 * knob, 4.0),
        );
        let (low, high) = (*range.start(), *range.end());
        if let Some(at) = slider.interact_pointer_pos() {
            if slider.dragged() || slider.clicked() {
                let part = ((at.x - track.left()) / track.width()).clamp(0.0, 1.0);
                let now = (low + part * (high - low)).round();
                if now != *value {
                    *value = now;
                    slider.mark_changed();
                }
            }
        }
        if slider.has_focus() {
            let (left, right) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::ArrowLeft),
                    input.key_pressed(egui::Key::ArrowRight),
                )
            });
            if left || right {
                let by = (high - low) / 20.0;
                *value = (*value + if right { by } else { -by }).clamp(low, high).round();
                slider.mark_changed();
            }
        }
        slider.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), ""));
        let part = ((*value - low) / (high - low).max(f32::EPSILON)).clamp(0.0, 1.0);
        let x = egui::lerp(track.left()..=track.right(), part);
        let enabled = ui.is_enabled();
        let hot = ui.ctx().animate_bool_responsive(
            slider.id.with("hot"),
            enabled && (slider.hovered() || slider.dragged() || slider.has_focus()),
        );
        let accent = if enabled { p.accent } else { p.faint };
        let painter = ui.painter();
        painter.rect_filled(track, 2.0, p.control_active());
        painter.rect_filled(
            egui::Rect::from_min_max(track.left_top(), egui::pos2(x, track.bottom())),
            2.0,
            accent,
        );
        let centre = egui::pos2(x, rect.center().y);
        if hot > 0.0 {
            painter.circle_filled(centre, knob + 5.0 * hot, accent.gamma_multiply(0.18 * hot));
        }
        painter.circle_filled(
            centre + egui::vec2(0.0, 1.0),
            knob,
            Color32::from_black_alpha(60),
        );
        painter.circle(
            centre,
            knob,
            if p.dark { p.text } else { Color32::WHITE },
            egui::Stroke::new(2.0, accent),
        );
        if let Some(said) = said {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, height), Sense::hover());
            ui.painter().text(
                rect.right_center(),
                egui::Align2::RIGHT_CENTER,
                said,
                egui::FontId::proportional(13.0),
                p.muted,
            );
        }
        (sign.clicked(), slider)
    });
    let (sign, slider) = inner.inner;
    LevelMoved { sign, slider }
}

/// One of several things to choose from, as a line of a list: a sign,
/// its name (cut where it is too long), a word or two about it at its
/// end, and a tick when it is the one chosen. As wide as its room.
pub fn choice_row(
    ui: &mut Ui,
    icon: Option<&str>,
    name: &str,
    about: Option<&str>,
    chosen: bool,
) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let ground = if chosen {
        theme::mix(p.control, p.accent, 0.26)
    } else if response.hovered() {
        p.control_hover()
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, ground);
    let mut left = rect.left() + 10.0;
    if let Some(icon) = icon {
        painter.text(
            egui::pos2(left + 8.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(15.0),
            if chosen { p.text } else { p.muted },
        );
        left += 24.0;
    }
    let mut right = rect.right() - 10.0;
    if chosen {
        painter.text(
            egui::pos2(right - 6.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            theme::icons::CHECK,
            egui::FontId::proportional(14.0),
            p.accent_fg(),
        );
        right -= 20.0;
    }
    if let Some(about) = about.filter(|about| !about.is_empty()) {
        let words = one_line(ui, about, egui::FontId::proportional(12.0), p.faint, 110.0);
        right -= words.size().x;
        ui.painter().galley(
            egui::pos2(right, rect.center().y - words.size().y / 2.0),
            words,
            p.faint,
        );
        right -= 8.0;
    }
    let words = one_line(ui, name, egui::FontId::proportional(13.0), p.text, right - left);
    ui.painter().galley(
        egui::pos2(left, rect.center().y - words.size().y / 2.0),
        words,
        p.text,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), chosen, name)
    });
    response
}

/// What is chosen of several things, in one line that opens the list of
/// them when it is pressed: what it is a choice of, the one chosen (cut
/// where it is too long), and an arrow that says whether the list is
/// open. The list itself is the caller's to draw under it.
pub fn chosen_line(ui: &mut Ui, of: &str, chosen: &str, open: bool) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        8.0,
        if response.hovered() {
            p.control_hover()
        } else {
            p.control
        },
        egui::Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    let label = one_line(ui, of, egui::FontId::proportional(12.0), p.muted, 96.0);
    let after = rect.left() + 10.0 + label.size().x + 10.0;
    ui.painter().galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - label.size().y / 2.0),
        label,
        p.muted,
    );
    let words = one_line(
        ui,
        chosen,
        egui::FontId::proportional(13.0),
        p.text,
        rect.right() - 30.0 - after,
    );
    ui.painter().galley(
        egui::pos2(after, rect.center().y - words.size().y / 2.0),
        words,
        p.text,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        if open {
            theme::icons::CARET_UP
        } else {
            theme::icons::CARET_DOWN
        },
        egui::FontId::proportional(13.0),
        p.muted,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), format!("{of}: {chosen}"))
    });
    response
}

/// A list that may be long, under what opened it: as high as its lines
/// up to `lines` of them, and scrolled beyond.
pub fn list_under<R>(
    ui: &mut Ui,
    id: &str,
    count: usize,
    lines: usize,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let row = 30.0 + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(row * count.clamp(1, lines.max(1)) as f32)
        .auto_shrink([false, true])
        .show(ui, add)
        .inner
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_written_stepped_and_spans_go_through_midnight() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(7 * 60 + 5), "07:05");
        assert_eq!(clock(DAY + 61), "01:01");
        assert_eq!(snapped(1202.4, 5), 1200);
        assert_eq!(snapped(1203.0, 5), 1205);
        assert_eq!(snapped(1438.0, 5), 0);
        assert_eq!(snapped(-4.0, 15), 0);
        // Evening to morning: nine hours; within a day; the whole day.
        assert_eq!(span(22 * 60, 7 * 60), 9 * 60);
        assert_eq!(span(9 * 60, 17 * 60 + 30), 8 * 60 + 30);
        assert_eq!(span(8 * 60, 8 * 60), DAY);
        assert_eq!(span_words(9 * 60), "9 h");
        assert_eq!(span_words(45), "45 min");
        assert_eq!(span_words(7 * 60 + 30), "7 h 30 min");
    }

    #[test]
    fn the_band_runs_from_noon_to_noon_with_the_night_in_its_middle() {
        assert_eq!(along_the_day(12 * 60), 0.0);
        assert_eq!(along_the_day(0), 0.5);
        assert_eq!(along_the_day(18 * 60), 0.25);
        assert_eq!(along_the_day(6 * 60), 0.75);
        // There and back.
        for minutes in [0u32, 5, 719, 720, 721, 1200, 1439] {
            let back = snapped(time_along(along_the_day(minutes)), 1);
            assert_eq!(back, minutes);
        }
        // A night is one piece; a span that holds noon is two.
        assert_eq!(band_parts(20 * 60, 7 * 60), [(1.0 / 3.0, 1.0 / 3.0 + 11.0 / 24.0)]);
        let over_noon = band_parts(10 * 60, 14 * 60);
        assert_eq!(over_noon.len(), 2);
        assert_eq!(over_noon[0].1, 1.0);
        assert_eq!(over_noon[1].0, 0.0);
        assert!((over_noon[1].1 - 2.0 / 24.0).abs() < 1e-5);
        assert_eq!(band_parts(300, 300), [(0.0, 1.0)]);
    }

    #[test]
    fn a_value_near_a_set_point_is_caught_by_it() {
        let stops = [
            Stop {
                value: 2000.0,
                name: "Candle",
            },
            Stop {
                value: 4000.0,
                name: "Warm",
            },
        ];
        let range = (1500.0, 6500.0);
        assert_eq!(caught(&stops, 4090.0, range), 4000.0);
        assert_eq!(caught(&stops, 3900.0, range), 4000.0);
        assert_eq!(caught(&stops, 3500.0, range), 3500.0);
        assert_eq!(stop_at(&stops, 4000.0, range), Some(1));
        assert_eq!(stop_at(&stops, 4030.0, range), Some(1));
        assert_eq!(stop_at(&stops, 4200.0, range), None);
        assert_eq!(caught(&[], 4090.0, range), 4090.0);
    }

    /// Each of them is drawn, in a frame without a window: nothing in
    /// them panics, and a press on a set point's name chooses it.
    #[test]
    fn they_draw_without_a_window() {
        let context = egui::Context::default();
        let (mut from, mut to, mut warmth, mut time) = (20 * 60, 7 * 60, 4000.0f32, 615u32);
        let stops = [Stop {
            value: 2700.0,
            name: "Lamp",
        }];
        let paint = |value: f32| Color32::from_gray((value / 30.0) as u8);
        for _ in 0..3 {
            let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    day_band(ui, &mut from, &mut to, &DayBand::default());
                    time_field(ui, &mut time);
                    ui.add(
                        StopSlider::new(&mut warmth, 1500.0..=6500.0)
                            .stops(&stops)
                            .painted(&paint)
                            .suffix(" K")
                            .step(50.0),
                    );
                    ui.add(StopSlider::new(&mut warmth, 1500.0..=6500.0));
                });
            });
            drawn.textures_delta.clear();
        }
        assert_eq!((from, to, time), (20 * 60, 7 * 60, 615));
        assert_eq!(warmth, 4000.0);
    }
}
