//! The widgets every gevrek program is built from: cards, setting rows, switches, sliders,
//! segmented controls, pills, key caps, colour fields. They take their colours from
//! [`theme::pal`], so every palette and accent applies everywhere.

use egui::{Color32, RichText, Sense, Ui};

use crate::color;
use crate::theme::{self, icons};

const LABEL_WIDTH: f32 = 250.0;

/// A titled card grouping related settings.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_frame(ui, |ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .size(15.5)
                .color(theme::pal(ui.ctx()).text),
        );
        ui.add_space(6.0);
        add(ui)
    })
}

/// The card surface without a title: a raised panel with a hairline border and soft shadow.
pub fn card_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::pal(ui.ctx());
    let radius = ui.visuals().window_corner_radius;
    ui.add_space(6.0);
    // Frame margins and stroke sit outside the content; size the content so the whole card fits.
    let margin = 18.0;
    let width = (ui.available_width() - 2.0 * margin - 4.0).max(100.0);
    let r = egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(radius)
        .inner_margin(margin)
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 40 } else { 10 }),
        })
        .show(ui, |ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner;
    ui.add_space(6.0);
    r
}

/// A small status dot (filled or hollow); egui's fonts lack ● and ○.
pub fn dot(ui: &mut Ui, filled: bool, color: Color32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    if filled {
        ui.painter().circle_filled(rect.center(), 3.5, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 3.0, egui::Stroke::new(1.0, color));
    }
    r
}

/// A toggle switch with a sliding knob.
pub fn switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let size = egui::vec2(38.0, 22.0);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, ""));
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let radius = rect.height() / 2.0;
    let off = if r.hovered() {
        p.control_active()
    } else {
        p.control_hover()
    };
    let bg = color::lerp(off, p.accent, t);
    let painter = ui.painter();
    if r.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            radius + 2.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Outside,
        );
    }
    painter.rect_filled(rect, radius, bg);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    let c = egui::pos2(x, rect.center().y);
    painter.circle_filled(
        c + egui::vec2(0.0, 1.0),
        radius - 3.0,
        Color32::from_black_alpha(50),
    );
    let knob = color::lerp(
        if p.dark { p.text } else { Color32::WHITE },
        p.on_accent(),
        t,
    );
    painter.circle_filled(c, radius - 3.0, knob);
    r
}

/// A borderless button showing a Phosphor icon, with a background on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> egui::Response {
    let r = ui.add(
        egui::Button::new(RichText::new(icon).size(16.0))
            .frame_when_inactive(false)
            .min_size(egui::vec2(30.0, 28.0)),
    );
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// A borderless button showing one of the brand's own icons (a window's
/// buttons), with a background on hover: as `icon_button`, which it falls
/// back to with `or` where the sign cannot be drawn.
pub fn brand_icon_button(
    ui: &mut Ui,
    icon: crate::brand_icons::BrandIcon,
    or: &str,
    tip: &str,
) -> egui::Response {
    let colour = ui.visuals().widgets.inactive.fg_stroke.color;
    let Some(image) = crate::brand_icons::image(ui.ctx(), icon, 16.0, colour) else {
        return icon_button(ui, or, tip);
    };
    let r = ui.add(
        egui::Button::image(image)
            .frame_when_inactive(false)
            .min_size(egui::vec2(30.0, 28.0)),
    );
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// A small icon button, as used for per-row actions.
pub fn small_icon_button(ui: &mut Ui, icon: &str, tip: &str) -> egui::Response {
    let p = theme::pal(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(icon).size(14.0).color(p.muted))
            .frame_when_inactive(false)
            .min_size(egui::vec2(24.0, 22.0)),
    )
    .on_hover_text(tip)
}

/// A frameless icon button for use with `add_enabled`.
pub fn icon_only(icon: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(icon).size(14.0))
        .frame_when_inactive(false)
        .min_size(egui::vec2(24.0, 22.0))
}

/// The main call to action: filled with the accent color.
pub fn primary_button(ui: &mut Ui, enabled: bool, text: impl Into<String>) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let text = text.into();
    let (fill, fg) = if enabled {
        (p.accent, p.on_accent())
    } else {
        (p.control, p.faint)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).strong().color(fg))
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

/// A rounded label with a tinted background, e.g. a status or a count.
pub fn pill(ui: &mut Ui, icon: &str, text: &str, color: Color32) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let label = if icon.is_empty() {
        text.to_owned()
    } else if text.is_empty() {
        icon.to_owned()
    } else {
        format!("{icon} {text}")
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, egui::FontId::proportional(12.5), color);
    let size = galley.size() + egui::vec2(18.0, 8.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let fill = p.tint(color, if r.hovered() { 0.22 } else { 0.14 });
    ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    r
}

/// A keyboard key cap, e.g. `Ctrl`.
pub fn kbd(ui: &mut Ui, key: &str) {
    let p = theme::pal(ui.ctx());
    let galley =
        ui.painter()
            .layout_no_wrap(key.to_owned(), egui::FontId::proportional(11.0), p.muted);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        4.0,
        p.control,
        egui::Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, p.muted);
}

/// Background for a clickable list row: tinted when selected, faint on hover.
pub fn list_row_bg(ui: &Ui, r: &egui::Response, rect: egui::Rect, selected: bool) {
    let p = theme::pal(ui.ctx());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect(
            rect,
            radius,
            p.tint(p.accent, 0.16),
            egui::Stroke::new(1.0, p.tint(p.accent, 0.45)),
            egui::StrokeKind::Inside,
        );
    } else if r.hovered() {
        ui.painter()
            .rect_filled(rect, radius, theme::mix(p.surface, p.text, 0.045));
    }
}

/// How wide [`keycaps`] paints a key combination, gap after it included:
/// to make room for it before it is painted.
pub fn keycaps_width(ui: &Ui, combo: &str) -> f32 {
    let painter = ui.painter();
    combo
        .split('+')
        .filter(|k| !k.is_empty())
        .map(|key| {
            painter
                .layout_no_wrap(
                    key.to_owned(),
                    egui::FontId::monospace(12.0),
                    Color32::WHITE,
                )
                .size()
                .x
                + 12.0
                + 4.0
        })
        .sum()
}

/// Paints a key combination such as `Mod+Shift+T` as key caps starting at `left`; returns the
/// x coordinate where they end.
pub fn keycaps(ui: &Ui, left: egui::Pos2, combo: &str, color: Color32) -> f32 {
    let p = theme::pal(ui.ctx());
    let painter = ui.painter();
    let mut x = left.x;
    for key in combo.split('+').filter(|k| !k.is_empty()) {
        let galley = painter.layout_no_wrap(key.to_owned(), egui::FontId::monospace(12.0), color);
        let size = egui::vec2(galley.size().x + 12.0, 20.0);
        let cap = egui::Rect::from_min_size(egui::pos2(x, left.y - size.y / 2.0), size);
        painter.rect_filled(cap.translate(egui::vec2(0.0, 1.5)), 4.0, p.border);
        painter.rect(
            cap,
            4.0,
            p.control,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        painter.galley(cap.center() - galley.size() / 2.0, galley, color);
        x = cap.right() + 4.0;
    }
    x
}

/// One line of a list of shortcuts: the keys as key caps, what they do,
/// and the mode they belong to where that is not the usual one. What
/// they do is broken into lines as wide as there is room for, and the
/// line grows with it: drawn on one line, a long one ran out of the
/// window. Where the keys leave too little room beside them, the words
/// go under them.
pub fn shortcut_row(ui: &mut Ui, keys: &str, action: &str, mode: Option<&str>) {
    let p = theme::pal(ui.ctx());
    let least = ui.spacing().interact_size.y + 4.0;
    let width = ui.available_width();
    let place = words_place(width, keycaps_width(ui, keys));
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = place.width;
    job.append(
        action,
        0.0,
        egui::TextFormat::simple(egui::FontId::proportional(14.0), p.text),
    );
    if let Some(mode) = mode {
        job.append(
            &format!("in the {mode} mode"),
            12.0,
            egui::TextFormat::simple(egui::FontId::proportional(12.0), p.accent_fg()),
        );
    }
    let words = ui.painter().layout_job(job);
    let first_line = words
        .rows
        .first()
        .map_or(words.size().y, |row| row.rect().height());
    let height = if place.under {
        CAP + GAP + words.size().y + 2.0 * PAD
    } else {
        (words.size().y + 2.0 * PAD).max(least)
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    list_row_bg(ui, &response, rect, false);
    // The keys stand by the words' first line, or above the words.
    let (words_top, keys_y) = if place.under {
        (rect.top() + PAD + CAP + GAP, rect.top() + PAD + CAP / 2.0)
    } else {
        let top = rect.top() + (height - words.size().y) / 2.0;
        (
            top,
            (top + first_line / 2.0).max(rect.top() + PAD + CAP / 2.0),
        )
    };
    keycaps(ui, egui::pos2(rect.left() + 6.0, keys_y), keys, p.text);
    ui.painter()
        .galley(egui::pos2(rect.left() + place.x, words_top), words, p.text);
}

/// The height of a key cap, and the room above, below and between.
const CAP: f32 = 20.0;
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;

/// Where the words of a shortcut's line go: from `x` (from the line's
/// left), at most `width` wide, beside the keys or `under` them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WordsPlace {
    x: f32,
    width: f32,
    under: bool,
}

/// The words stand in a column of their own, 230 points in (nearer in a
/// narrow window), or after the keys where those are wider. Where that
/// leaves them too little room to be read, they go under the keys,
/// across the whole line.
fn words_place(row: f32, keys: f32) -> WordsPlace {
    const COLUMN: f32 = 230.0;
    const LEAST: f32 = 160.0;
    let x = (6.0 + keys + 10.0).max(COLUMN.min(row / 2.0));
    let beside = row - x - 6.0;
    if beside >= LEAST {
        WordsPlace {
            x,
            width: beside,
            under: false,
        }
    } else {
        WordsPlace {
            x: 6.0,
            width: (row - 12.0).max(1.0),
            under: true,
        }
    }
}

/// A checkbox in the app's style: a rounded box filled with the accent and a drawn check when on.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let text = text.into();
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let size = egui::vec2(
        18.0 + 8.0 + galley.size().x,
        ui.spacing().interact_size.y.max(galley.size().y),
    );
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, galley.text())
    });
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let bx = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 9.0, rect.center().y),
        egui::vec2(18.0, 18.0),
    );
    let painter = ui.painter();
    let off_fill = if r.hovered() {
        p.control_active()
    } else {
        p.control_hover()
    };
    painter.rect(
        bx,
        5.0,
        color::lerp(off_fill, p.accent, t),
        egui::Stroke::new(1.0, color::lerp(p.border_strong(), p.accent, t)),
        egui::StrokeKind::Inside,
    );
    if t > 0.0 {
        // The check draws itself in as the box fills.
        let a = bx.left_center() + egui::vec2(4.5, 0.5);
        let b = bx.center_bottom() + egui::vec2(-1.5, -5.0);
        let c = bx.right_top() + egui::vec2(-4.0, 5.0);
        let stroke = egui::Stroke::new(2.0, p.on_accent());
        let first = t.min(0.5) * 2.0;
        painter.line_segment([a, a + (b - a) * first], stroke);
        if t > 0.5 {
            painter.line_segment([b, b + (c - b) * ((t - 0.5) * 2.0)], stroke);
        }
    }
    if r.has_focus() {
        painter.rect_stroke(
            bx.expand(2.0),
            7.0,
            egui::Stroke::new(2.0, p.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Outside,
        );
    }
    let color = if ui.is_enabled() { p.text } else { p.faint };
    painter.galley(
        egui::pos2(bx.right() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    r
}

/// A slider in the app's style: a thin track filled with the accent up to a round knob, and an
/// editable value beside it. Mirrors the parts of `egui::Slider`'s builder the pages use.
pub struct Slider<'a, N: egui::emath::Numeric> {
    value: &'a mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: String,
    max_decimals: Option<usize>,
    fixed_decimals: Option<usize>,
    clamp: bool,
    log: bool,
}

impl<'a, N: egui::emath::Numeric> Slider<'a, N> {
    pub fn new(value: &'a mut N, range: std::ops::RangeInclusive<N>) -> Self {
        Self {
            value,
            range,
            suffix: String::new(),
            max_decimals: None,
            fixed_decimals: None,
            clamp: true,
            log: false,
        }
    }
    pub fn suffix(mut self, s: impl Into<String>) -> Self {
        self.suffix = s.into();
        self
    }
    pub fn max_decimals(mut self, n: usize) -> Self {
        self.max_decimals = Some(n);
        self
    }
    pub fn fixed_decimals(mut self, n: usize) -> Self {
        self.fixed_decimals = Some(n);
        self
    }
    /// `Never` lets the value box take values past the track's range.
    pub fn clamping(mut self, c: egui::SliderClamping) -> Self {
        self.clamp = !matches!(c, egui::SliderClamping::Never);
        self
    }
    pub fn logarithmic(mut self, on: bool) -> Self {
        self.log = on;
        self
    }

    fn decimals(&self) -> Option<usize> {
        if N::INTEGRAL {
            Some(0)
        } else {
            self.fixed_decimals.or(self.max_decimals)
        }
    }
}

impl<N: egui::emath::Numeric> egui::Widget for Slider<'_, N> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let p = theme::pal(ui.ctx());
        let (min, max) = (self.range.start().to_f64(), self.range.end().to_f64());
        let log = self.log && min > 0.0 && max > min;
        let to_t = |v: f64| -> f64 {
            let t = if log {
                (v / min).ln() / (max / min).ln()
            } else {
                (v - min) / (max - min).max(f64::EPSILON)
            };
            t.clamp(0.0, 1.0)
        };
        let from_t = |t: f64| -> f64 {
            if log {
                min * (max / min).powf(t)
            } else {
                min + t * (max - min)
            }
        };
        let decimals = self.decimals();
        let round = |v: f64| match decimals {
            Some(d) => {
                let m = 10f64.powi(d as i32);
                (v * m).round() / m
            }
            None => v,
        };
        let mut v = self.value.to_f64();

        let inner = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let size = egui::vec2(ui.spacing().slider_width, ui.spacing().interact_size.y);
            let (rect, mut r) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let knob_r = 8.0;
            let track = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width() - 2.0 * knob_r, 4.0),
            );
            if let Some(pos) = r.interact_pointer_pos() {
                if r.dragged() || r.clicked() {
                    let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
                    let nv = round(from_t(t));
                    if nv != v {
                        v = nv;
                        r.mark_changed();
                    }
                }
            }
            if r.has_focus() {
                let (left, right) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                    )
                });
                if left || right {
                    let step = if N::INTEGRAL {
                        1.0
                    } else {
                        (max - min) / 100.0
                    };
                    let nv = if log {
                        from_t((to_t(v) + if right { 0.01 } else { -0.01 }).clamp(0.0, 1.0))
                    } else {
                        (v + if right { step } else { -step }).clamp(min, max)
                    };
                    v = round(nv);
                    r.mark_changed();
                }
            }
            r.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), v, ""));

            let enabled = ui.is_enabled();
            let t = to_t(v) as f32;
            let x = egui::lerp(track.left()..=track.right(), t);
            let hot = ui.ctx().animate_bool_responsive(
                r.id.with("hot"),
                enabled && (r.hovered() || r.dragged() || r.has_focus()),
            );
            let painter = ui.painter();
            let accent = if enabled { p.accent } else { p.faint };
            painter.rect_filled(track, 2.0, p.control_active());
            painter.rect_filled(
                egui::Rect::from_min_max(track.left_top(), egui::pos2(x, track.bottom())),
                2.0,
                accent,
            );
            let c = egui::pos2(x, rect.center().y);
            if hot > 0.0 {
                painter.circle_filled(c, knob_r + 5.0 * hot, accent.gamma_multiply(0.18 * hot));
            }
            painter.circle_filled(
                c + egui::vec2(0.0, 1.0),
                knob_r,
                Color32::from_black_alpha(60),
            );
            painter.circle(
                c,
                knob_r,
                if p.dark { p.text } else { Color32::WHITE },
                egui::Stroke::new(2.0, accent),
            );

            let mut dv = egui::DragValue::new(&mut v)
                .speed(if log { 0.0 } else { (max - min) / 300.0 })
                .suffix(self.suffix.clone());
            if self.clamp {
                dv = dv.range(min..=max);
            }
            if let Some(d) = decimals {
                dv = if self.fixed_decimals.is_some() || N::INTEGRAL {
                    dv.fixed_decimals(d)
                } else {
                    dv.max_decimals(d)
                };
            }
            let dr = ui.add_sized([62.0, size.y], dv);
            r.union(dr)
        });
        let mut r = inner.inner;
        if v != self.value.to_f64() {
            *self.value = N::from_f64(v);
            r.mark_changed();
        }
        r
    }
}

/// A line of text to type into, as wide as said: the one kind of text
/// field every page has.
pub fn text_field(ui: &mut Ui, text: &mut String, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).desired_width(width))
}

/// A field to search by: a magnifier and what is looked for in its
/// place until something is typed.
pub fn search_field(
    ui: &mut Ui,
    text: &mut String,
    looked_for: &str,
    width: f32,
) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(format!("{}  {looked_for}", icons::MAGNIFYING_GLASS))
            .desired_width(width),
    )
}

/// A number by itself, where no range is known that a slider could be
/// drawn for: dragged or typed, in a box as wide as a slider's own.
pub fn number_box<N: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut N,
    speed: f64,
) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    ui.add_sized([62.0, height], egui::DragValue::new(value).speed(speed))
}

/// One of many to choose from, more than fit side by side: the one
/// chosen, and the list of all of them when it is pressed. Each has its
/// name and, where there is something to say of it, words that come up
/// when the pointer rests on it. Returns the one that was chosen.
pub fn dropdown<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    chosen: &T,
    options: &[(T, String, String)],
    width: f32,
) -> Option<T> {
    let shown = options
        .iter()
        .find(|(value, _, _)| value == chosen)
        .map_or(String::new(), |(_, name, _)| name.clone());
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for (value, name, about) in options {
                let is = value == chosen;
                let line = ui.selectable_label(is, name);
                let line = if about.is_empty() {
                    line
                } else {
                    line.on_hover_text(about)
                };
                if line.clicked() && !is {
                    picked = Some(value.clone());
                }
            }
        });
    picked
}

/// What a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    File,
    Folder,
    /// A program: a file, or only its name, which the system looks for
    /// where it looks for programs (nothing is said of a name alone).
    Program,
}

/// How a path that was typed stands on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFound {
    /// Nothing is typed.
    Empty,
    /// It is there and is what it should be.
    There,
    /// Something is there that is the other kind.
    OtherKind,
    Missing,
}

/// A path as it is typed made whole: `~` at its start is the home folder.
pub fn whole_path(typed: &str, home: Option<&std::path::Path>) -> std::path::PathBuf {
    let typed = typed.trim();
    match (typed.strip_prefix("~/").or(typed.strip_prefix("~\\")), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if typed == "~" => home.map_or_else(|| typed.into(), std::path::Path::to_path_buf),
        _ => typed.into(),
    }
}

/// Whether what a path names is there.
pub fn path_found(typed: &str, kind: PathKind, home: Option<&std::path::Path>) -> PathFound {
    if typed.trim().is_empty() {
        return PathFound::Empty;
    }
    // (A program by its name alone is the system's to find.)
    if kind == PathKind::Program && !typed.contains(['/', '\\']) {
        return PathFound::Empty;
    }
    let path = whole_path(typed, home);
    match (kind, path.is_file(), path.is_dir()) {
        (PathKind::File | PathKind::Program, true, _) | (PathKind::Folder, _, true) => {
            PathFound::There
        }
        (_, false, false) => PathFound::Missing,
        _ => PathFound::OtherKind,
    }
}

/// A path to type: the text, and beside it a sign that says whether the
/// file or the folder is there on this machine (the words that come up
/// over the sign say what was looked for), and a button that opens
/// gevrek's own browser to choose it. `home`: the home folder, for a
/// path that begins with `~`.
pub fn path_field(
    ui: &mut Ui,
    text: &mut String,
    kind: PathKind,
    home: Option<&std::path::Path>,
    width: f32,
) -> egui::Response {
    // (A program is looked for among programs; any other file among all.)
    let (name, kinds): (&str, &[&str]) = match kind {
        PathKind::Program => ("Programs", crate::browser::PROGRAMS),
        _ => ("", &[]),
    };
    path_field_of(ui, text, kind, home, width, name, kinds)
}

/// A path to type, as [`path_field`], whose browser shows the files of
/// these kinds (by the ends of their names; `kinds_name` is the word for
/// them: "Pictures").
pub fn path_field_of(
    ui: &mut Ui,
    text: &mut String,
    kind: PathKind,
    home: Option<&std::path::Path>,
    width: f32,
    kinds_name: &str,
    kinds: &[&str],
) -> egui::Response {
    use crate::browser::{Browser, Done};
    use std::sync::{Arc, Mutex};
    let p = theme::pal(ui.ctx());
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let mut field = text_field(ui, text, width);
        // The browser: opened by the button, kept by the toolkit while it
        // is open (under this field's own name), and what is chosen in
        // it is the field's text.
        let open_id = field.id.with("its browser");
        let what_for = match kind {
            PathKind::File => "Choose a file…",
            PathKind::Folder => "Choose a folder…",
            PathKind::Program => "Choose a program…",
        };
        if small_icon_button(ui, icons::FOLDER_OPEN, what_for).clicked() {
            let browser = Browser::new(kind, text, home, kinds_name, kinds);
            ui.data_mut(|data| data.insert_temp(open_id, Arc::new(Mutex::new(browser))));
        }
        let open: Option<Arc<Mutex<Browser>>> = ui.data(|data| data.get_temp(open_id));
        if let Some(browser) = open {
            let done = browser
                .lock()
                .map(|mut browser| browser.show(ui.ctx()))
                .unwrap_or(Some(Done::Cancelled));
            if let Some(done) = done {
                ui.data_mut(|data| data.remove::<Arc<Mutex<Browser>>>(open_id));
                if let Done::Chosen(path) = done {
                    *text = path.display().to_string();
                    field.mark_changed();
                }
            }
        }
        let what = match kind {
            PathKind::File => "file",
            PathKind::Folder => "folder",
            PathKind::Program => "program",
        };
        let whole = whole_path(text, home);
        let (sign, colour, says) = match path_found(text, kind, home) {
            PathFound::Empty => return field,
            PathFound::There => (
                icons::CHECK_CIRCLE,
                p.ok,
                format!("{} is there on this machine.", whole.display()),
            ),
            PathFound::OtherKind => (
                icons::WARNING,
                p.warn,
                format!("{} is there, and is no {what}.", whole.display()),
            ),
            PathFound::Missing => (
                icons::WARNING,
                p.warn,
                format!("There is no {what} {} on this machine.", whole.display()),
            ),
        };
        ui.label(RichText::new(sign).size(15.0).color(colour))
            .on_hover_text(says);
        field
    });
    inner.inner
}

/// A button that is not the one thing to do here: as high as every
/// control, with the plain look.
pub fn secondary_button(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    ui.add(egui::Button::new(text.into()).min_size(egui::vec2(0.0, height)))
}

/// A button that is only its words (and a sign before them): for what is
/// done seldom and should not weigh on the page.
pub fn text_button(ui: &mut Ui, icon: &str, text: &str) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let words = if icon.is_empty() {
        text.to_string()
    } else {
        format!("{icon}  {text}")
    };
    ui.add(egui::Button::new(RichText::new(words).size(12.5).color(p.muted)).frame(false))
}

/// The sign that something is under way.
pub fn busy(ui: &mut Ui) -> egui::Response {
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
    ui.add(egui::Spinner::new().size(14.0))
}

/// A line that says something about the page it stands on (it is being
/// tried, something is wrong, something was done): tinted in the colour
/// of what it says, with a sign.
pub fn banner<R>(ui: &mut Ui, colour: Color32, icon: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::pal(ui.ctx());
    egui::Frame::new()
        .fill(p.tint(colour, 0.14))
        .stroke(egui::Stroke::new(1.0, p.tint(colour, 0.45)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            // (What it says may be long, and may have buttons after it:
            // it goes on in the next line where the page is narrow.)
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon).size(16.0).color(colour));
                add(ui)
            })
            .inner
        })
        .inner
}
/// A segmented control: options side by side in one pill, the selected one raised on a sliding
/// highlight. `dim` draws the highlight faintly, for a value that is the default rather than
/// set. Returns the option clicked, if any.
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: egui::Id,
    selected: Option<&T>,
    options: &[(T, &str)],
    dim: bool,
) -> Option<T> {
    let p = theme::pal(ui.ctx());
    let font = egui::FontId::proportional(13.5);
    let pad = 14.0;
    let h = ui.spacing().interact_size.y;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, l)| {
            ui.painter()
                .layout_no_wrap((*l).to_owned(), font.clone(), p.text)
                .size()
                .x
                + 2.0 * pad
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total, h), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius.nw as f32 + 2.0;
    ui.painter().rect(
        rect,
        radius,
        p.control,
        egui::Stroke::new(1.0, theme::mix(p.control, p.border, 0.6)),
        egui::StrokeKind::Inside,
    );

    let mut x = rect.left() + 3.0;
    let mut slots = Vec::with_capacity(options.len());
    for w in &widths {
        slots.push(egui::Rect::from_min_size(
            egui::pos2(x, rect.top() + 3.0),
            egui::vec2(*w, h - 6.0),
        ));
        x += w;
    }
    let sel = selected.and_then(|s| options.iter().position(|(v, _)| v == s));
    if let Some(i) = sel {
        let target = slots[i];
        let left = ui
            .ctx()
            .animate_value_with_time(id.with("x"), target.left(), 0.16);
        let width = ui
            .ctx()
            .animate_value_with_time(id.with("w"), target.width(), 0.16);
        let hl = egui::Rect::from_min_size(
            egui::pos2(left, target.top()),
            egui::vec2(width, target.height()),
        );
        let fill = if dim {
            theme::mix(p.control, p.surface, 0.5)
        } else if p.dark {
            p.control_active()
        } else {
            p.surface
        };
        if !dim {
            ui.painter().rect_filled(
                hl.translate(egui::vec2(0.0, 1.0)),
                radius - 2.0,
                Color32::from_black_alpha(if p.dark { 70 } else { 22 }),
            );
        }
        ui.painter().rect_filled(hl, radius - 2.0, fill);
    }
    let mut out = None;
    for (i, ((value, label), slot)) in options.iter().zip(&slots).enumerate() {
        let r = ui.interact(*slot, id.with(i), Sense::click());
        let is_sel = sel == Some(i);
        let color = if (is_sel && !dim) || r.hovered() {
            p.text
        } else {
            p.muted
        };
        ui.painter().text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            color,
        );
        if r.clicked() {
            out = Some(value.clone());
        }
    }
    out
}

/// An uppercase caption above a group of items.
pub fn caption(ui: &mut Ui, text: &str) {
    let p = theme::pal(ui.ctx());
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .strong()
            .color(p.faint)
            .extra_letter_spacing(0.8),
    );
}

/// Row layout: label on the left (with help on hover), control on the right, reset at the end.
pub fn row<R>(
    ui: &mut Ui,
    label: &str,
    help: &str,
    is_set: bool,
    highlight: bool,
    control: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    let mut reset = false;
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.horizontal(|ui| {
        // The name's room: the usual width, so that the controls of a
        // page stand under each other; more for a name that is longer,
        // as far as leaves the control its place. A name longer still is
        // cut, and said whole when the pointer rests on it: it is never
        // drawn under its control.
        let usual = LABEL_WIDTH.min(ui.available_width() * 0.4);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let needed = ui
            .painter()
            .layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER)
            .size()
            .x
            + 26.0;
        let most = (ui.available_width() - 170.0).max(usual);
        let label_width = needed.clamp(usual, most);
        let cut = needed > label_width;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(label_width, ui.spacing().interact_size.y),
            Sense::hover(),
        );
        let text = if is_set {
            RichText::new(label)
        } else {
            RichText::new(label).weak()
        };
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if is_set {
            let c = theme::pal(child.ctx()).accent_fg();
            dot(&mut child, true, c).on_hover_text("Changed from the default");
        } else {
            let c = child.visuals().weak_text_color();
            dot(&mut child, false, c).on_hover_text("At its default");
        }
        let l = child.add(egui::Label::new(text).truncate());
        match (cut, help.is_empty()) {
            (true, true) => {
                l.on_hover_text(label);
            }
            (true, false) => {
                l.on_hover_text(format!("{label}\n{help}"));
            }
            (false, false) => {
                l.on_hover_text(help);
            }
            (false, true) => {}
        }
        let out = control(ui);
        if is_set
            && small_icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Back to the default")
                .clicked()
        {
            reset = true;
        }
        out
    });
    // A faint band behind the hovered row ties the label to its control.
    let band = inner.response.rect.expand2(egui::vec2(8.0, 2.0));
    if ui.rect_contains_pointer(band) {
        let p = theme::pal(ui.ctx());
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(band, 6.0, theme::mix(p.surface, p.text, 0.035)),
        );
    }
    if highlight {
        ui.painter().rect_stroke(
            inner.response.rect.expand(3.0),
            4.0,
            egui::Stroke::new(2.0, theme::pal(ui.ctx()).accent_fg()),
            egui::StrokeKind::Outside,
        );
        inner.response.scroll_to_me(Some(egui::Align::Center));
    }
    (inner.inner, reset)
}

/// A color swatch with picker plus a text field accepting any CSS color. Returns the new string.
pub fn color_field(ui: &mut Ui, id: egui::Id, current: Option<&str>) -> Option<String> {
    color_field_or(ui, id, current, None)
}

/// Like [`color_field`]; when unset, the swatch shows `fallback` (the colour in effect).
pub fn color_field_or(
    ui: &mut Ui,
    id: egui::Id,
    current: Option<&str>,
    fallback: Option<Color32>,
) -> Option<String> {
    let parsed = current.and_then(color::parse);
    let mut c = parsed
        .or(fallback)
        .unwrap_or(Color32::from_rgb(127, 200, 255));
    let mut out = None;
    let mut rgba = c.to_srgba_unmultiplied();
    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
        c = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
        out = Some(color::format(c));
    }
    // Text keeps whatever the user typed (e.g. "red") until it parses.
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.unwrap_or("").to_owned());
    let r = ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(110.0)
            .hint_text("#rrggbb"),
    );
    if r.changed() {
        if color::parse(&text).is_some() {
            out = Some(text.clone());
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    } else if !r.has_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    if current.is_some() && parsed.is_none() {
        ui.colored_label(ui.visuals().warn_fg_color, icons::WARNING)
            .on_hover_text("Not a colour (try #rrggbb, rgb(...), or a name)");
    }
    out
}

/// Page heading with an optional subtitle.
pub fn heading(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = theme::pal(ui.ctx());
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(26.0).strong().color(p.text));
    if !subtitle.is_empty() {
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).size(14.0).color(p.muted));
    }
    ui.add_space(10.0);
}

/// Splits a command line into arguments, honoring single and double quotes and backslashes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                in_word = true;
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

/// Joins arguments into a command line that [`split_args`] reads back identically.
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if !a.is_empty() && !a.chars().any(|c| c.is_whitespace() || "\"'\\".contains(c)) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\"'\"'"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Shortens text to roughly fit `width` points, adding an ellipsis.
pub fn ellipsize(s: &str, width: f32) -> String {
    let max = ((width / 7.2) as usize).max(4);
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!(
            "{}…",
            s.chars().take(max - 1).collect::<String>().trim_end()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_of_a_shortcut_have_room_beside_the_keys_or_go_under_them() {
        // Short keys: the words' own column, the rest of the line wide.
        let place = words_place(604.0, 80.0);
        assert_eq!((place.x, place.under), (230.0, false));
        assert_eq!(place.width, 604.0 - 230.0 - 6.0);
        // Wide keys: after them.
        let place = words_place(604.0, 300.0);
        assert_eq!((place.x, place.under), (316.0, false));
        // Keys that leave too little: under them, across the line.
        let place = words_place(604.0, 480.0);
        assert!(place.under);
        assert_eq!((place.x, place.width), (6.0, 592.0));
        // A narrow window: the column comes nearer.
        assert!(!words_place(400.0, 60.0).under);
    }

    #[test]
    fn a_path_is_made_whole_and_looked_for() {
        let home = std::env::temp_dir();
        assert_eq!(
            whole_path("~/pictures/a.png", Some(&home)),
            home.join("pictures/a.png")
        );
        assert_eq!(whole_path("~", Some(&home)), home);
        assert_eq!(
            whole_path(" /abs/a.png ", Some(&home)),
            std::path::PathBuf::from("/abs/a.png")
        );
        assert_eq!(whole_path("~/a", None), std::path::PathBuf::from("~/a"));
        // The folder of temporary files is there, and is a folder.
        let there = home.to_string_lossy().to_string();
        assert_eq!(path_found(&there, PathKind::Folder, None), PathFound::There);
        assert_eq!(
            path_found(&there, PathKind::File, None),
            PathFound::OtherKind
        );
        assert_eq!(path_found("  ", PathKind::File, None), PathFound::Empty);
        assert_eq!(
            path_found("no/such/thing/anywhere.png", PathKind::File, None),
            PathFound::Missing
        );
    }

    #[test]
    fn args_round_trip() {
        let args = vec![
            "noctalia".to_owned(),
            "msg".into(),
            "a b".into(),
            "it's".into(),
            String::new(),
        ];
        assert_eq!(split_args(&join_args(&args)), args);
        assert_eq!(
            split_args(r#"sh -c "echo \"hi\"""#),
            vec!["sh", "-c", "echo \"hi\""]
        );
    }
}
