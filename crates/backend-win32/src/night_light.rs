//! The night light on Windows: at its hours every display is given a
//! colour ramp with less blue (and green) in it, and its own again
//! afterwards. The arithmetic is the model's (`gevrek_model::night_light`),
//! as on Linux; what is Windows' own is here.
//!
//! - A display's own ramp (it may hold a calibration) is read before it is
//!   first changed; the warm ramp is that one made smaller, colour by
//!   colour, and the own one is given back when the night light ends, when
//!   the window manager stops, and when it is dropped without stopping.
//! - Windows takes no ramp that is far from the display's usual one (about
//!   half of it is the least): a temperature that asks for less blue than
//!   that gets the least Windows takes, and the log says so once.
//! - Windows gives a display its usual ramp back on its own account (a
//!   change of the displays, a program that had the whole screen): what the
//!   displays have is looked at again and again, and put right.
//! - A display that takes no ramp (some in HDR, a virtual one) is left; if
//!   none takes it, that is a notice.
//! - The colours do not change at once: they turn warm, and back, over a
//!   moment (`TURN`), slowly at first and at the end, as on Linux
//!   (`gevrek_model::night_light::between`). While they turn, the window
//!   manager's loop looks at them every frame.
//!
//! Windows has a night light of its own (Settings, Display). The two do not
//! know of each other; both on is warmer than either.

use gevrek_engine::overlay_data::Notice;
use gevrek_model::night_light::{between, factors, NightLightConfig, NEUTRAL};
use std::time::{Duration, Instant};

/// How long the colours take from one temperature to another.
const TURN: Duration = Duration::from_millis(1800);

/// The colours on their way from one temperature to another.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Turn {
    from: u32,
    to: u32,
    since: Instant,
}

impl Turn {
    /// The temperature at this moment, and whether the way is over.
    fn at(&self, now: Instant) -> (u32, bool) {
        let part = now.saturating_duration_since(self.since).as_secs_f32() / TURN.as_secs_f32();
        (between(self.from, self.to, part), part >= 1.0)
    }
}

/// A display's ramp as Windows keeps it: 256 steps each of red, green and
/// blue.
type Ramp = [u16; 768];

/// The least of a colour Windows lets a ramp keep, with a little room.
const LEAST: f32 = 0.52;

/// A display's own ramp with each colour made smaller.
fn warmed(own: &Ramp, (red, green, blue): (f32, f32, f32)) -> Ramp {
    let mut ramp = *own;
    for (colour, factor) in [red, green, blue].into_iter().enumerate() {
        for step in &mut ramp[colour * 256..(colour + 1) * 256] {
            *step = (f32::from(*step) * factor.clamp(0.0, 1.0)).round() as u16;
        }
    }
    ramp
}

/// The ramp of a display that nothing has changed: every colour from
/// none to all of it, in even steps.
fn plain_ramp() -> Ramp {
    let mut ramp = [0u16; 768];
    for (at, step) in ramp.iter_mut().enumerate() {
        *step = ((at % 256) * 257) as u16;
    }
    ramp
}

/// Whether a ramp found on a display reads as one a night light left
/// there: its green or its blue ends well below its red. A gevrek that is
/// ended at once (not stopped) cannot give the displays their own ramps
/// back; the next one would take the warm ramp for the display's own,
/// warm it again, and give the warm one "back" at its end.
fn left_warm(ramp: &Ramp) -> bool {
    let (red, green, blue) = (
        u32::from(ramp[255]),
        u32::from(ramp[511]),
        u32::from(ramp[767]),
    );
    red > 0 && (green * 100 < red * 93 || blue * 100 < red * 93)
}

/// The factors as near to what is asked as Windows takes.
fn within_what_windows_takes((red, green, blue): (f32, f32, f32)) -> (f32, f32, f32) {
    (red.max(LEAST), green.max(LEAST), blue.max(LEAST))
}

/// What is said when the night light is asked for and no display takes it.
pub fn notice(config: &NightLightConfig, impossible: bool) -> Option<Notice> {
    (config.enabled && impossible).then(|| Notice {
        id: "night-light-impossible".to_string(),
        title: "The night light cannot change the displays' colours".to_string(),
        text: "The night light is switched on (desktop.night_light) and no display takes a \
               colour ramp from gevrek: a display in HDR and a virtual display take none. \
               Windows has a night light of its own (Settings, System, Display)."
            .to_string(),
    })
}

/// One display that was given a ramp.
struct Warm {
    /// Windows' name of it (`\\.\DISPLAY1`).
    device: String,
    own: Ramp,
    given: Ramp,
}

/// The night light as it is on the displays now.
#[derive(Default)]
pub struct NightLight {
    /// The temperature the displays are at; `None`: their own colours.
    kelvin: Option<u32>,
    warm: Vec<Warm>,
    /// It is asked for and no display takes it.
    pub impossible: bool,
    /// Said once: Windows takes less than what was asked.
    said_least: bool,
    /// The temperature the colours are to end at (daylight: their own),
    /// once it was asked for.
    aim: Option<u32>,
    /// They are on their way there.
    turn: Option<Turn>,
    /// A temperature that is only shown to be looked at, and until when
    /// (`DispatchCommand::NightLightPreview`).
    preview: Option<(u32, Instant)>,
}

impl NightLight {
    /// Make the displays what the configuration asks for at this time of
    /// the day (minutes since midnight). Cheap when nothing is to be done:
    /// while it is on, each display is asked for its ramp. `true` when
    /// whether it is possible changed (a notice comes or goes).
    pub fn follow(&mut self, config: &NightLightConfig, now: u32) -> bool {
        self.follow_at(config, now, Instant::now())
    }

    /// Whether the colours are on their way from one temperature to
    /// another, or a temperature is shown that goes by itself: they are
    /// then to be looked at every frame.
    pub fn turning(&self) -> bool {
        self.turn.is_some() || self.preview.is_some()
    }

    /// Show this temperature for so many seconds, whatever the
    /// configuration says, and then what it says again.
    pub fn preview(&mut self, temperature: u32, seconds: u32) {
        self.preview = Some((
            gevrek_model::night_light::within(temperature),
            Instant::now()
                + Duration::from_secs(gevrek_model::night_light::preview_seconds(seconds)),
        ));
    }

    /// The temperature the colours are to end at now: the one that is
    /// only shown, while its time lasts; else the configuration's at this
    /// time of the day (daylight: the displays' own colours).
    fn aim_at(&mut self, config: &NightLightConfig, now: u32, moment: Instant) -> u32 {
        if let Some((kelvin, until)) = self.preview {
            if moment < until {
                return kelvin;
            }
            self.preview = None;
        }
        if config.on_at(now) {
            config.kelvin()
        } else {
            NEUTRAL
        }
    }

    fn follow_at(&mut self, config: &NightLightConfig, now: u32, moment: Instant) -> bool {
        // Where the colours are to end: at a temperature, or (daylight)
        // at their own. Another end than before: they set out for it
        // from where they are.
        let aim = self.aim_at(config, now, moment);
        if self.aim != Some(aim) {
            let from = match self.turn {
                Some(turn) => turn.at(moment).0,
                None => self.kelvin.unwrap_or(NEUTRAL),
            };
            self.aim = Some(aim);
            self.turn = (from != aim).then_some(Turn {
                from,
                to: aim,
                since: moment,
            });
        }
        let level = match self.turn {
            Some(turn) => {
                let (level, over) = turn.at(moment);
                if over {
                    self.turn = None;
                }
                level
            }
            None => aim,
        };
        let wanted = (level != NEUTRAL).then_some(level);
        let was_impossible = self.impossible;
        match wanted {
            None => {
                if self.kelvin.is_some() || !self.warm.is_empty() {
                    self.restore();
                    log::info!("Night light off: the displays have their own colours");
                }
                self.impossible = false;
            }
            Some(kelvin) => {
                let asked = factors(kelvin);
                let taken = within_what_windows_takes(asked);
                if asked != taken && !self.said_least {
                    self.said_least = true;
                    log::warn!(
                        "Night light: {kelvin} K asks for less of a colour than Windows lets a \
                         display's ramp keep; the displays get the least it takes"
                    );
                }
                let devices = system::displays();
                // A display that went is forgotten (its ramp went with it).
                self.warm.retain(|warm| devices.contains(&warm.device));
                let mut changed = 0;
                for device in &devices {
                    let Some(now_has) = system::ramp_of(device) else {
                        continue;
                    };
                    let at = self.warm.iter().position(|warm| &warm.device == device);
                    let own = match at {
                        // Still as it was given, at this temperature.
                        Some(at)
                            if self.kelvin == Some(kelvin) && self.warm[at].given == now_has =>
                        {
                            continue
                        }
                        // What it has is ours (another temperature now), or
                        // Windows gave it its usual one back.
                        Some(at) => self.warm[at].own,
                        None if left_warm(&now_has) => {
                            log::info!(
                                "Night light: the display {device} still has a warm ramp from a gevrek that was ended; its own is taken to be the plain one"
                            );
                            plain_ramp()
                        }
                        None => now_has,
                    };
                    let given = warmed(&own, taken);
                    if !system::give(device, &given) {
                        log::debug!("the display {device} did not take a colour ramp");
                        continue;
                    }
                    changed += 1;
                    match at {
                        Some(at) => self.warm[at].given = given,
                        None => self.warm.push(Warm {
                            device: device.clone(),
                            own,
                            given,
                        }),
                    }
                }
                self.impossible = !devices.is_empty() && self.warm.is_empty();
                // (Said where they end, not at every step of the way.)
                if changed > 0 && self.turn.is_none() {
                    log::info!(
                        "Night light: {changed} of {} displays at {kelvin} K",
                        devices.len()
                    );
                }
                // Nothing takes a ramp: there is no way to go.
                if self.warm.is_empty() {
                    self.turn = None;
                }
                if self.impossible && !was_impossible {
                    log::warn!("The night light is asked for and no display takes a colour ramp");
                }
                self.kelvin = Some(kelvin);
            }
        }
        self.impossible != was_impossible
    }

    /// Every display gets its own ramp back.
    pub fn restore(&mut self) {
        for warm in self.warm.drain(..) {
            if !system::give(&warm.device, &warm.own) {
                log::warn!(
                    "the display {} did not take its own colour ramp back",
                    warm.device
                );
            }
        }
        self.kelvin = None;
        self.aim = None;
        self.turn = None;
    }
}

impl Drop for NightLight {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Minutes since midnight, by the machine's clock.
pub fn minutes_now() -> u32 {
    let now = jiff::Zoned::now();
    now.hour() as u32 * 60 + now.minute() as u32
}

/// Daylight changes nothing: kept so that the arithmetic above and the
/// model's agree on what "off" is.
const _: () = assert!(NEUTRAL == 6500);

mod system {
    use super::Ramp;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateDCW, DeleteDC, EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
        DISPLAY_DEVICE_MIRRORING_DRIVER,
    };
    use windows::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};

    /// The displays that are part of the desktop, by Windows' names.
    pub fn displays() -> Vec<String> {
        let mut found = Vec::new();
        for index in 0.. {
            let mut device = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            // SAFETY: the structure is ours and says how large it is.
            if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
                break;
            }
            if device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0
                || device.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0
            {
                continue;
            }
            let length = device
                .DeviceName
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(device.DeviceName.len());
            found.push(String::from_utf16_lossy(&device.DeviceName[..length]));
        }
        found
    }

    fn with_display<T>(
        device: &str,
        then: impl FnOnce(windows::Win32::Graphics::Gdi::HDC) -> T,
    ) -> Option<T> {
        let name: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the name ends with a zero and outlives the call; the
        // device context is deleted below.
        unsafe {
            let context = CreateDCW(PCWSTR(name.as_ptr()), PCWSTR(name.as_ptr()), None, None);
            if context.is_invalid() {
                return None;
            }
            let result = then(context);
            let _ = DeleteDC(context);
            Some(result)
        }
    }

    /// The ramp a display has now.
    pub fn ramp_of(device: &str) -> Option<Ramp> {
        let mut ramp: Ramp = [0; 768];
        // SAFETY: the ramp is as large as Windows writes (3 x 256 words).
        with_display(device, |context| unsafe {
            GetDeviceGammaRamp(context, ramp.as_mut_ptr().cast()).as_bool()
        })?
        .then_some(ramp)
    }

    /// Give a display a ramp; `false` when it does not take it.
    pub fn give(device: &str, ramp: &Ramp) -> bool {
        // SAFETY: as above; Windows only reads the ramp.
        with_display(device, |context| unsafe {
            SetDeviceGammaRamp(context, ramp.as_ptr().cast()).as_bool()
        })
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_ramp_left_warm_is_not_taken_for_a_displays_own() {
        let plain = super::plain_ramp();
        assert_eq!((plain[0], plain[255], plain[256], plain[767]), (0, 65535, 0, 65535));
        assert!(!super::left_warm(&plain));
        // As the night light leaves it: less green, much less blue.
        assert!(super::left_warm(&super::warmed(&plain, (1.0, 0.8, 0.6))));
        // A display's own correction of a few percent is its own.
        assert!(!super::left_warm(&super::warmed(&plain, (1.0, 0.97, 0.95))));
    }

    use super::*;

    fn straight() -> Ramp {
        let mut ramp = [0u16; 768];
        for colour in 0..3 {
            for step in 0..256 {
                ramp[colour * 256 + step] = (step * 257) as u16;
            }
        }
        ramp
    }

    #[test]
    fn a_warm_ramp_is_the_displays_own_with_less_blue() {
        let own = straight();
        let warm = warmed(&own, factors(4000));
        // Red stays, green is less, blue is least; nothing is lighter.
        assert_eq!(warm[255], 65535);
        assert!(warm[511] < 65535 && warm[767] < warm[511]);
        assert!(warm.iter().zip(own.iter()).all(|(warm, own)| warm <= own));
        // Black stays black, and each colour still rises.
        assert_eq!((warm[0], warm[256], warm[512]), (0, 0, 0));
        assert!(warm[512..768].windows(2).all(|pair| pair[0] <= pair[1]));
        // Daylight changes nothing; a calibration is kept in proportion.
        assert_eq!(warmed(&own, factors(NEUTRAL)), own);
        let mut calibrated = own;
        calibrated[600] = 20000;
        assert_eq!(warmed(&calibrated, (1.0, 1.0, 0.5))[600], 10000);
    }

    #[test]
    fn the_colours_turn_over_a_moment_and_end_where_they_are_asked() {
        let since = Instant::now();
        let turn = Turn {
            from: NEUTRAL,
            to: 4000,
            since,
        };
        assert_eq!(turn.at(since), (NEUTRAL, false));
        let (half, over) = turn.at(since + TURN / 2);
        assert!(half < NEUTRAL && half > 4000 && !over, "{half}");
        // Slowly at first: a tenth of the time is less than a tenth of the way.
        let (early, _) = turn.at(since + TURN / 10);
        assert!(NEUTRAL - early < (NEUTRAL - 4000) / 10, "{early}");
        assert_eq!(turn.at(since + TURN), (4000, true));
        assert_eq!(turn.at(since + TURN * 3), (4000, true));
        // And back the same way.
        let back = Turn {
            from: 4000,
            to: NEUTRAL,
            since,
        };
        assert_eq!(back.at(since + TURN), (NEUTRAL, true));
        assert!(back.at(since + TURN / 2).0 > 4000);
    }

    #[test]
    fn a_temperature_that_is_only_shown_goes_by_itself() {
        let mut light = NightLight::default();
        let mut config = NightLightConfig::default();
        let moment = Instant::now();
        // Off, at noon: the displays' own colours.
        assert_eq!(light.aim_at(&config, 12 * 60, moment), NEUTRAL);
        light.preview(3000, 4);
        assert!(light.turning());
        assert_eq!(light.aim_at(&config, 12 * 60, moment), 3000);
        // Its time over: what the configuration says, and it is forgotten.
        let later = moment + Duration::from_secs(40);
        assert_eq!(light.aim_at(&config, 12 * 60, later), NEUTRAL);
        assert!(light.preview.is_none());
        // Over a night light that is on: back to that one.
        config.enabled = true;
        config.from.clear();
        config.to.clear();
        light.preview(2500, 99);
        assert_eq!(light.aim_at(&config, 12 * 60, moment), 2500);
        assert_eq!(light.aim_at(&config, 12 * 60, later), config.kelvin());
        // What no display can show is brought within what one can.
        light.preview(100, 1);
        assert_eq!(light.aim_at(&config, 0, moment), 1000);
    }

    #[test]
    fn no_colour_is_asked_to_be_less_than_windows_takes() {
        let candle = within_what_windows_takes(factors(1500));
        assert!(candle.2 >= LEAST && candle.1 >= LEAST && candle.0 >= 0.99);
        // What Windows takes anyway is as it was asked.
        assert_eq!(within_what_windows_takes(factors(5000)), factors(5000));
    }

    #[test]
    fn it_is_only_spoken_of_when_it_is_on_and_cannot_be_done() {
        let mut config = NightLightConfig::default();
        assert!(notice(&config, true).is_none());
        config.enabled = true;
        assert!(notice(&config, false).is_none());
        assert_eq!(
            notice(&config, true).map(|notice| notice.id),
            Some("night-light-impossible".to_string())
        );
    }
}
