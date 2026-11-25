//! The screen shared or recorded: who is told, and what is kept out.
//!
//! gevrek is the compositor, so every picture another program takes of a
//! display is drawn here (`capture.rs`, `image_capture.rs`), whichever
//! way it asks: a screenshot tool, a recorder, the screen-sharing portal.
//! Two things follow that a shell's surface with something private on it
//! (the icons of the desktop, a notification) can rest on:
//!
//! - **It is told.** A display of which pictures are taken again and
//!   again is shared or recorded; one picture is a screenshot, and is
//!   not. When that begins and when it ends the event `screen.shared`
//!   goes out ([`Sharing`]), and a surface can cover itself meanwhile.
//! - **It can be left out.** A layer rule of the configuration with
//!   `"exclude_from_capture": true` keeps the surfaces of that name out
//!   of every picture taken of a display, the single screenshot too:
//!   they are drawn on the display and not in the picture
//!   ([`kept_out_of_captures`]).
//!
//! A picture of one window never shows a shell's surface, and counts
//! for neither.

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use smithay::reexports::wayland_server::Resource as _;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Two pictures of a display within this time: it is being shared.
const AGAIN_WITHIN: Duration = Duration::from_millis(2500);
/// No picture for this long, and none waited for: it is over.
const OVER_AFTER: Duration = Duration::from_millis(3000);

/// What changed of the sharing of a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change<K> {
    Started(K),
    Stopped(K),
}

struct Watch {
    last: Instant,
    shared: bool,
}

/// Which displays pictures are taken of again and again.
pub struct Sharing<K> {
    watched: HashMap<K, Watch>,
}

impl<K> Default for Sharing<K> {
    fn default() -> Self {
        Self {
            watched: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash + Ord> Sharing<K> {
    /// A picture of a display was handed to a program. Whether that
    /// makes the display a shared one: the second picture in a short
    /// while does.
    pub fn picture(&mut self, of: K, now: Instant) -> Option<Change<K>> {
        match self.watched.get_mut(&of) {
            Some(watch) if now.saturating_duration_since(watch.last) <= AGAIN_WITHIN => {
                watch.last = now;
                if watch.shared {
                    return None;
                }
                watch.shared = true;
                Some(Change::Started(of))
            }
            Some(watch) if watch.shared => {
                // (Long since the last one, and still shared: a picture
                // was waited for all the while.)
                watch.last = now;
                None
            }
            _ => {
                self.watched.insert(
                    of,
                    Watch {
                        last: now,
                        shared: false,
                    },
                );
                None
            }
        }
    }

    /// Time went by: the displays that are shared no longer. `waited`
    /// says of a display whether a program waits for its next picture (a
    /// recorder that asks only when something changes waits as long as
    /// the display stands still, and is recording all the while).
    pub fn tick(&mut self, now: Instant, waited: impl Fn(K) -> bool) -> Vec<Change<K>> {
        let mut over: Vec<K> = self
            .watched
            .iter()
            .filter(|(of, watch)| {
                now.saturating_duration_since(watch.last) > OVER_AFTER && !waited(**of)
            })
            .map(|(of, _)| *of)
            .collect();
        over.sort();
        over.into_iter()
            .filter_map(|of| {
                self.watched
                    .remove(&of)
                    .filter(|watch| watch.shared)
                    .map(|_| Change::Stopped(of))
            })
            .collect()
    }

    /// The displays that are shared now, in order.
    pub fn shared(&self) -> Vec<K> {
        let mut shared: Vec<K> = self
            .watched
            .iter()
            .filter(|(_, watch)| watch.shared)
            .map(|(of, _)| *of)
            .collect();
        shared.sort();
        shared
    }

    /// Whether anything is looked at (time has to be told).
    pub fn watches(&self) -> bool {
        !self.watched.is_empty()
    }
}

/// Whether the surfaces of this name are kept out of pictures other
/// programs take of a display: their layer rule says
/// `"exclude_from_capture": true`. (Read from the rule's own keys that
/// the configuration's model does not name yet; it keeps them as they
/// are written.)
pub fn kept_out_of_captures(
    rules: &[gevrek_model::layer_rules::LayerRule],
    namespace: &str,
) -> bool {
    gevrek_model::layer_rules::rule_for(rules, namespace).is_some_and(|rule| {
        rule.extra.0.get("exclude_from_capture") == Some(&serde_json::Value::Bool(true))
    })
}

impl State {
    /// A picture of a display was handed to another program.
    pub(crate) fn picture_taken(&mut self, of: MonitorHandle) {
        if let Some(change) = self.sharing.picture(of, Instant::now()) {
            self.tell_sharing(change);
        }
        // (Its end is looked for by the host's passes.)
        self.host_soon();
    }

    /// Look whether a display is shared no longer. Called from the
    /// host's passes; `now` is a parameter so that tests can move time.
    pub fn tick_sharing(&mut self, now: Instant) {
        if !self.sharing.watches() {
            return;
        }
        let waited: Vec<MonitorHandle> = self
            .captures
            .iter()
            .filter(|capture| capture.frame.is_alive())
            .map(|capture| capture.output)
            .chain(
                self.image_captures
                    .iter()
                    .filter_map(|capture| capture.output_waited_for()),
            )
            .collect();
        for change in self.sharing.tick(now, |of| waited.contains(&of)) {
            self.tell_sharing(change);
        }
    }

    /// The names of the displays that are shared or recorded now.
    pub fn shared_displays(&self) -> Vec<String> {
        self.sharing
            .shared()
            .into_iter()
            .filter_map(|of| self.display_name(of))
            .collect()
    }

    fn display_name(&self, of: MonitorHandle) -> Option<String> {
        self.outputs
            .iter()
            .find(|(id, _)| *id == of)
            .map(|(_, output)| output.name())
    }

    /// `screen.shared`: `state` is `started` or `stopped`, `output` the
    /// display it is said of, and `shared` every display that is shared
    /// now, their names with commas between (empty: none), so that who
    /// hears one event knows all there is to know.
    fn tell_sharing(&mut self, change: Change<MonitorHandle>) {
        let (state, of) = match change {
            Change::Started(of) => ("started", of),
            Change::Stopped(of) => ("stopped", of),
        };
        let output = self.display_name(of).unwrap_or_default();
        log::info!("The pictures other programs take of {output}: {state}");
        self.emit(
            gevrek_engine::executor::RuntimeEventKind::ScreenShared,
            None,
            vec![
                ("state".to_string(), state.to_string()),
                ("output".to_string(), output),
                ("shared".to_string(), self.shared_displays().join(",")),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn one_picture_is_a_screenshot_and_the_second_soon_after_is_sharing() {
        let start = Instant::now();
        let mut sharing: Sharing<u32> = Sharing::default();
        assert_eq!(sharing.picture(1, start), None);
        assert!(sharing.shared().is_empty());
        assert_eq!(
            sharing.picture(1, after(start, 40)),
            Some(Change::Started(1))
        );
        // Said once, however many pictures follow.
        assert_eq!(sharing.picture(1, after(start, 80)), None);
        assert_eq!(sharing.shared(), [1]);
        // Another display is another matter.
        assert_eq!(sharing.picture(2, after(start, 90)), None);
        assert_eq!(sharing.shared(), [1]);
    }

    #[test]
    fn a_screenshot_now_and_another_much_later_are_two_screenshots() {
        let start = Instant::now();
        let mut sharing: Sharing<u32> = Sharing::default();
        assert_eq!(sharing.picture(1, start), None);
        assert_eq!(sharing.picture(1, after(start, 20_000)), None);
        assert!(sharing.shared().is_empty());
        // (And a third soon after the second is sharing.)
        assert_eq!(
            sharing.picture(1, after(start, 20_500)),
            Some(Change::Started(1))
        );
    }

    #[test]
    fn sharing_is_over_when_no_picture_is_taken_or_waited_for() {
        let start = Instant::now();
        let mut sharing: Sharing<u32> = Sharing::default();
        sharing.picture(1, start);
        sharing.picture(1, after(start, 30));
        // Soon after the last picture: still shared.
        assert!(sharing.tick(after(start, 1000), |_| false).is_empty());
        // A recorder waits for the display to change: shared for as
        // long as it waits.
        assert!(sharing.tick(after(start, 60_000), |of| of == 1).is_empty());
        assert_eq!(sharing.shared(), [1]);
        // Its next picture, long after: nothing new to tell.
        assert_eq!(sharing.picture(1, after(start, 61_000)), None);
        // Nobody waits any more.
        assert_eq!(
            sharing.tick(after(start, 65_000), |_| false),
            [Change::Stopped(1)]
        );
        assert!(sharing.shared().is_empty());
        assert!(!sharing.watches());
    }

    #[test]
    fn a_screenshot_is_forgotten_without_a_word() {
        let start = Instant::now();
        let mut sharing: Sharing<u32> = Sharing::default();
        sharing.picture(7, start);
        assert!(sharing.watches());
        assert!(sharing.tick(after(start, 10_000), |_| false).is_empty());
        assert!(!sharing.watches());
    }

    #[test]
    fn a_rule_keeps_surfaces_of_its_name_out_of_pictures() {
        let rules: Vec<gevrek_model::layer_rules::LayerRule> =
            serde_json::from_value(serde_json::json!([
                { "namespace": "gevrek-zones*", "exclude_from_capture": true },
                { "namespace": "gevrek-panel", "blur": true },
                { "namespace": "waybar", "exclude_from_capture": "yes" },
            ]))
            .expect("rules");
        assert!(kept_out_of_captures(&rules, "gevrek-zones"));
        assert!(kept_out_of_captures(&rules, "gevrek-zones.dev"));
        assert!(!kept_out_of_captures(&rules, "gevrek-panel"));
        // Only a plain yes counts.
        assert!(!kept_out_of_captures(&rules, "waybar"));
        assert!(!kept_out_of_captures(&rules, "rofi"));
        assert!(!kept_out_of_captures(&[], "gevrek-zones"));
    }
}
