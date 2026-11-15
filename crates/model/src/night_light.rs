//! Warmer colours in the evening ("night light"): the displays show less
//! blue between two times of the day, or for as long as it is switched on.
//!
//! The arithmetic is here, for every backend: whether it is on at a time
//! of the day, what a colour temperature means for red, green and blue,
//! and the ramp a display is given for it.

use serde::{Deserialize, Serialize};

/// Daylight, by which nothing is changed.
pub const NEUTRAL: u32 = 6500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NightLightConfig {
    pub enabled: bool,
    /// The colour temperature while it is on, in kelvin: 6500 is
    /// daylight (no change), 4000 is warm, 2500 is candle light.
    pub temperature: u32,
    /// From when in the evening (`20:30`), by the clock of the machine.
    /// Empty, like `to`: the whole day.
    pub from: String,
    /// Until when in the morning (`07:00`).
    pub to: String,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            temperature: 4000,
            from: "20:00".to_string(),
            to: "07:00".to_string(),
        }
    }
}

/// A time of the day as `HH:MM`, in minutes since midnight.
pub fn minutes(text: &str) -> Option<u32> {
    let (hours, rest) = text.trim().split_once(':')?;
    let (hours, rest): (u32, u32) = (hours.trim().parse().ok()?, rest.trim().parse().ok()?);
    (hours < 24 && rest < 60).then_some(hours * 60 + rest)
}

/// A temperature within what a display can show.
pub fn within(temperature: u32) -> u32 {
    temperature.clamp(1000, NEUTRAL)
}

/// How long a temperature that is only shown to be looked at
/// (`DispatchCommand::NightLightPreview`) stays, in seconds, whatever was
/// asked: long enough to see, never so long that it is taken for a fault.
pub fn preview_seconds(asked: u32) -> u64 {
    u64::from(asked.clamp(1, 30))
}

impl NightLightConfig {
    /// The temperature that is asked for, within what a display can show.
    pub fn kelvin(&self) -> u32 {
        within(self.temperature)
    }

    /// Whether the colours are warm at this time of the day (minutes
    /// since midnight). The span may cross midnight; with no times, or
    /// times that cannot be read, it is the whole day.
    pub fn on_at(&self, now: u32) -> bool {
        if !self.enabled {
            return false;
        }
        match (minutes(&self.from), minutes(&self.to)) {
            (Some(from), Some(to)) if from == to => true,
            (Some(from), Some(to)) if from < to => (from..to).contains(&now),
            (Some(from), Some(to)) => now >= from || now < to,
            _ => true,
        }
    }

    /// What is said when it was switched on or off by its key: that it
    /// is on, and, when its hours are not now, from when the colours
    /// turn warm (switched on in the afternoon it changes nothing yet,
    /// and saying nothing would look like doing nothing).
    pub fn said_when_switched(&self, now: u32) -> String {
        if !self.enabled {
            return "Night light off".to_string();
        }
        if self.on_at(now) {
            return "Night light on".to_string();
        }
        format!("Night light on from {}", self.from.trim())
    }

    /// What cannot be right, as `(path, message)`.
    pub fn problems(&self) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        for (name, text) in [("from", &self.from), ("to", &self.to)] {
            if !text.trim().is_empty() && minutes(text).is_none() {
                problems.push((
                    format!("desktop.night_light.{name}"),
                    format!("'{text}' is not a time of the day as HH:MM"),
                ));
            }
        }
        if self.from.trim().is_empty() != self.to.trim().is_empty() {
            problems.push((
                "desktop.night_light".to_string(),
                "Give both times, or neither (for the whole day)".to_string(),
            ));
        }
        problems
    }
}

/// How much of red, green and blue is left at a colour temperature, each
/// from 0 to 1 (all 1 at daylight). After Tanner Helland's fit of the
/// colour of a glowing body, normalised so that daylight changes nothing.
pub fn factors(kelvin: u32) -> (f32, f32, f32) {
    let of = |kelvin: f32| -> (f32, f32, f32) {
        let t = kelvin / 100.0;
        let red = if t <= 66.0 {
            255.0
        } else {
            329.698_73 * (t - 60.0).powf(-0.133_204_76)
        };
        let green = if t <= 66.0 {
            99.470_8 * t.ln() - 161.119_57
        } else {
            288.122_17 * (t - 60.0).powf(-0.075_514_85)
        };
        let blue = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_73 * (t - 10.0).ln() - 305.044_8
        };
        (
            red.clamp(0.0, 255.0),
            green.clamp(0.0, 255.0),
            blue.clamp(0.0, 255.0),
        )
    };
    let kelvin = kelvin.clamp(1000, NEUTRAL) as f32;
    let (red, green, blue) = of(kelvin);
    let (red_day, green_day, blue_day) = of(NEUTRAL as f32);
    (
        (red / red_day).min(1.0),
        (green / green_day).min(1.0),
        (blue / blue_day).min(1.0),
    )
}

/// The ramp a display is given for one colour: `size` steps from nothing
/// to `factor` of full.
/// The colour on the way from one to another, in kelvin: `part` of the
/// way is gone (0 to 1), slowly at first and slowly at the end, in steps
/// of 25 kelvin (no eye sees finer ones); at the end it is `to` exactly.
pub fn between(from: u32, to: u32, part: f32) -> u32 {
    if part >= 1.0 {
        return to;
    }
    let part = part.max(0.0);
    let eased = part * part * (3.0 - 2.0 * part);
    let kelvin = (from as f32 + (to as f32 - from as f32) * eased).round() as u32;
    kelvin / 25 * 25
}

pub fn ramp(size: usize, factor: f32) -> Vec<u16> {
    let last = size.saturating_sub(1).max(1) as f32;
    (0..size)
        .map(|step| (step as f32 / last * factor.clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_goes_from_one_to_the_other_without_a_jump() {
        // From the display's own colour to a warm one and back: it begins
        // where it was, ends exactly where it is going, never leaves the
        // range between the two, and no step is larger than a twentieth
        // of the way when looked at sixty times.
        for (from, to) in [(NEUTRAL, 3400), (3400, NEUTRAL), (4000, 4000)] {
            assert_eq!(between(from, to, 0.0), from / 25 * 25);
            assert_eq!(between(from, to, 1.0), to);
            assert_eq!(between(from, to, 7.0), to);
            let (low, high) = (from.min(to), from.max(to));
            let mut before = between(from, to, 0.0);
            for step in 1..=60 {
                let now = between(from, to, step as f32 / 60.0);
                assert!(now >= low / 25 * 25 && now <= high, "{now} between {low} and {high}");
                assert!(now.abs_diff(before) <= (high - low) / 20 + 25, "{before} to {now}");
                before = now;
            }
        }
    }

    #[test]
    fn switching_it_on_outside_its_hours_says_when_it_begins() {
        let mut night = NightLightConfig {
            enabled: true,
            from: "20:00".to_string(),
            to: "07:00".to_string(),
            ..Default::default()
        };
        assert_eq!(
            night.said_when_switched(19 * 60 + 23),
            "Night light on from 20:00"
        );
        assert_eq!(night.said_when_switched(21 * 60), "Night light on");
        assert_eq!(night.said_when_switched(3 * 60), "Night light on");
        night.enabled = false;
        assert_eq!(night.said_when_switched(21 * 60), "Night light off");
        // No hours: the whole day.
        let always = NightLightConfig {
            enabled: true,
            from: String::new(),
            to: String::new(),
            ..Default::default()
        };
        assert_eq!(always.said_when_switched(12 * 60), "Night light on");
    }

    #[test]
    fn it_is_on_between_its_times_also_across_midnight() {
        let mut night = NightLightConfig {
            enabled: true,
            ..Default::default()
        };
        let at = |text: &str| minutes(text).unwrap();
        assert!(night.on_at(at("23:10")) && night.on_at(at("02:00")) && night.on_at(at("20:00")));
        assert!(
            !night.on_at(at("07:00")) && !night.on_at(at("12:00")) && !night.on_at(at("19:59"))
        );
        // A span within one day.
        night.from = "09:00".to_string();
        night.to = "17:30".to_string();
        assert!(night.on_at(at("12:00")) && !night.on_at(at("18:00")) && !night.on_at(at("03:00")));
        // No times: the whole day. Switched off: never.
        night.from.clear();
        night.to.clear();
        assert!(night.on_at(at("12:00")) && night.problems().is_empty());
        night.enabled = false;
        assert!(!night.on_at(at("23:00")));
        // What cannot be read is said.
        night.from = "25:00".to_string();
        assert_eq!(night.problems().len(), 2, "{:?}", night.problems());
        assert_eq!(minutes("7:05"), Some(425));
        assert_eq!(minutes("7"), None);
    }

    #[test]
    fn warmer_is_less_blue_and_daylight_changes_nothing() {
        let day = factors(NEUTRAL);
        assert!(
            (day.0 - 1.0).abs() < 1e-3 && (day.1 - 1.0).abs() < 1e-3 && (day.2 - 1.0).abs() < 1e-3
        );
        let warm = factors(4000);
        let candle = factors(2500);
        assert!((warm.0 - 1.0).abs() < 1e-3, "red stays: {warm:?}");
        assert!(warm.1 < 1.0 && warm.2 < warm.1, "{warm:?}");
        assert!(
            candle.1 < warm.1 && candle.2 < warm.2,
            "{candle:?} {warm:?}"
        );
        assert!(candle.2 > 0.0 && candle.2 < 0.5);
        // Outside what makes sense it is the nearest that does.
        assert_eq!(factors(9000), factors(NEUTRAL));
        assert_eq!(factors(10), factors(1000));
    }

    #[test]
    fn a_ramp_runs_from_nothing_to_its_factor_of_full() {
        let full = ramp(256, 1.0);
        assert_eq!((full[0], full[255], full.len()), (0, 65535, 256));
        let half = ramp(1024, 0.5);
        assert_eq!((half[0], half[1023]), (0, 32768));
        assert!(half.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(ramp(0, 1.0).is_empty());
    }
}
