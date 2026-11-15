use gevrek_model::settings::GestureDirection;

#[derive(Debug, Clone, PartialEq)]
pub struct TouchpadContact {
    pub id: u32,
    /// Device-normalized horizontal coordinate in the inclusive 0..=1 range.
    pub x: f32,
    /// Device-normalized vertical coordinate in the inclusive 0..=1 range.
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchpadFrame {
    pub contacts: Vec<TouchpadContact>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureRegistration {
    pub id: i32,
    pub fingers: u8,
    pub direction: GestureDirection,
    pub distance_percent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    Started,
    Updated,
    Completed,
    Cancelled,
}

impl GesturePhase {
    pub fn config_name(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Updated => "updated",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureEvent {
    pub registration_ids: Vec<i32>,
    pub phase: GesturePhase,
    pub fingers: u8,
    pub direction: Option<GestureDirection>,
    pub progress: f32,
    pub delta_x: f32,
    pub delta_y: f32,
}

#[derive(Debug, Clone)]
struct ActiveGesture {
    fingers: u8,
    start_x: f32,
    start_y: f32,
    direction: Option<GestureDirection>,
    progress: f32,
    delta_x: f32,
    delta_y: f32,
    last_frame_ms: u64,
    committed: bool,
}

/// Device-independent swipe recognizer. The Windows backend supplies contact
/// coordinates normalized from each touchpad's HID logical range.
#[derive(Debug, Clone)]
pub struct GestureRecognizer {
    registrations: Vec<GestureRegistration>,
    idle_timeout_ms: u32,
    axis_lock_ratio: f32,
    active: Option<ActiveGesture>,
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new(350, 1.25)
    }
}

impl GestureRecognizer {
    pub fn new(idle_timeout_ms: u32, axis_lock_ratio: f32) -> Self {
        Self {
            registrations: Vec::new(),
            idle_timeout_ms,
            axis_lock_ratio,
            active: None,
        }
    }

    pub fn configure(
        &mut self,
        registrations: Vec<GestureRegistration>,
        idle_timeout_ms: u32,
        axis_lock_ratio: f32,
    ) {
        self.registrations = registrations;
        self.idle_timeout_ms = idle_timeout_ms;
        self.axis_lock_ratio = axis_lock_ratio;
        self.active = None;
    }

    pub fn feed_frame(&mut self, frame: TouchpadFrame, timestamp_ms: u64) -> Vec<GestureEvent> {
        let mut events = Vec::new();
        let fingers = frame.contacts.len().min(255) as u8;
        if fingers == 0 {
            if let Some(event) = self.cancel_active() {
                events.push(event);
            }
            return events;
        }

        let Some((centroid_x, centroid_y)) = centroid(&frame.contacts) else {
            return events;
        };

        if self
            .active
            .as_ref()
            .is_some_and(|active| active.fingers != fingers)
        {
            if let Some(event) = self.cancel_active() {
                events.push(event);
            }
        }

        if self.active.is_none() {
            if !self
                .registrations
                .iter()
                .any(|registration| registration.fingers == fingers)
            {
                return events;
            }
            self.active = Some(ActiveGesture {
                fingers,
                start_x: centroid_x,
                start_y: centroid_y,
                direction: None,
                progress: 0.0,
                delta_x: 0.0,
                delta_y: 0.0,
                last_frame_ms: timestamp_ms,
                committed: false,
            });
            events.push(GestureEvent {
                registration_ids: self.ids_for(fingers, None),
                phase: GesturePhase::Started,
                fingers,
                direction: None,
                progress: 0.0,
                delta_x: 0.0,
                delta_y: 0.0,
            });
            return events;
        }

        let minimum_distance = self
            .registrations
            .iter()
            .filter(|registration| registration.fingers == fingers)
            .map(|registration| registration.distance_percent / 100.0)
            .reduce(f32::min)
            .unwrap_or(0.18);

        let registrations = &self.registrations;
        let active = self.active.as_mut().expect("active gesture initialized");
        active.last_frame_ms = timestamp_ms;
        active.delta_x = centroid_x - active.start_x;
        active.delta_y = centroid_y - active.start_y;

        if active.committed {
            return events;
        }

        if active.direction.is_none() {
            let dead_zone = (minimum_distance * 0.08).clamp(0.004, 0.025);
            active.direction = direction_from_delta(
                active.delta_x,
                active.delta_y,
                self.axis_lock_ratio,
                dead_zone,
            );
        }

        let Some(direction) = active.direction else {
            return events;
        };
        let directional_distance = match direction {
            GestureDirection::Left => (-active.delta_x).max(0.0),
            GestureDirection::Right => active.delta_x.max(0.0),
            GestureDirection::Up => (-active.delta_y).max(0.0),
            GestureDirection::Down => active.delta_y.max(0.0),
        };
        let threshold = registrations
            .iter()
            .filter(|registration| {
                registration.fingers == fingers && registration.direction == direction
            })
            .map(|registration| registration.distance_percent / 100.0)
            .reduce(f32::min)
            .unwrap_or(minimum_distance)
            .max(f32::EPSILON);
        let raw_progress = directional_distance / threshold;
        active.progress = raw_progress.clamp(0.0, 1.0);
        let ids = registrations
            .iter()
            .filter(|registration| {
                registration.fingers == fingers && registration.direction == direction
            })
            .map(|registration| registration.id)
            .collect::<Vec<_>>();
        let phase = if raw_progress >= 0.999 && !ids.is_empty() {
            active.committed = true;
            active.progress = 1.0;
            GesturePhase::Completed
        } else {
            GesturePhase::Updated
        };
        events.push(GestureEvent {
            registration_ids: ids,
            phase,
            fingers,
            direction: Some(direction),
            progress: active.progress,
            delta_x: active.delta_x,
            delta_y: active.delta_y,
        });
        events
    }

    pub fn tick(&mut self, timestamp_ms: u64) -> Option<GestureEvent> {
        let timed_out = self.active.as_ref().is_some_and(|active| {
            timestamp_ms.saturating_sub(active.last_frame_ms) >= u64::from(self.idle_timeout_ms)
        });
        timed_out.then(|| self.cancel_active()).flatten()
    }

    fn cancel_active(&mut self) -> Option<GestureEvent> {
        let active = self.active.take()?;
        if active.committed {
            return None;
        }
        Some(GestureEvent {
            registration_ids: self.ids_for(active.fingers, active.direction),
            phase: GesturePhase::Cancelled,
            fingers: active.fingers,
            direction: active.direction,
            progress: active.progress,
            delta_x: active.delta_x,
            delta_y: active.delta_y,
        })
    }

    fn ids_for(&self, fingers: u8, direction: Option<GestureDirection>) -> Vec<i32> {
        self.registrations
            .iter()
            .filter(|registration| {
                registration.fingers == fingers
                    && direction.is_none_or(|direction| registration.direction == direction)
            })
            .map(|registration| registration.id)
            .collect()
    }
}

fn centroid(contacts: &[TouchpadContact]) -> Option<(f32, f32)> {
    if contacts.is_empty() {
        return None;
    }
    let (sum_x, sum_y) = contacts
        .iter()
        .fold((0.0, 0.0), |(x, y), contact| (x + contact.x, y + contact.y));
    let count = contacts.len() as f32;
    Some((sum_x / count, sum_y / count))
}

fn direction_from_delta(
    delta_x: f32,
    delta_y: f32,
    axis_lock_ratio: f32,
    dead_zone: f32,
) -> Option<GestureDirection> {
    let horizontal = delta_x.abs();
    let vertical = delta_y.abs();
    if horizontal.max(vertical) < dead_zone {
        return None;
    }
    if horizontal >= vertical * axis_lock_ratio {
        Some(if delta_x < 0.0 {
            GestureDirection::Left
        } else {
            GestureDirection::Right
        })
    } else if vertical >= horizontal * axis_lock_ratio {
        Some(if delta_y < 0.0 {
            GestureDirection::Up
        } else {
            GestureDirection::Down
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(id: i32, fingers: u8, direction: GestureDirection) -> GestureRegistration {
        GestureRegistration {
            id,
            fingers,
            direction,
            distance_percent: 20.0,
        }
    }

    fn frame(points: &[(f32, f32)]) -> TouchpadFrame {
        TouchpadFrame {
            contacts: points
                .iter()
                .enumerate()
                .map(|(id, (x, y))| TouchpadContact {
                    id: id as u32,
                    x: *x,
                    y: *y,
                })
                .collect(),
        }
    }

    #[test]
    fn three_finger_swipe_commits_once_at_the_threshold() {
        let mut recognizer = GestureRecognizer::default();
        recognizer.configure(vec![registration(7, 3, GestureDirection::Left)], 350, 1.25);
        assert_eq!(
            recognizer.feed_frame(frame(&[(0.6, 0.5), (0.5, 0.5), (0.4, 0.5)]), 0)[0].phase,
            GesturePhase::Started
        );
        let updated = recognizer.feed_frame(frame(&[(0.5, 0.5), (0.4, 0.5), (0.3, 0.5)]), 16);
        assert_eq!(updated[0].phase, GesturePhase::Updated);
        assert!((updated[0].progress - 0.5).abs() < 0.001);
        let committed = recognizer.feed_frame(frame(&[(0.4, 0.5), (0.3, 0.5), (0.2, 0.5)]), 32);
        assert_eq!(committed[0].phase, GesturePhase::Completed);
        assert_eq!(committed[0].registration_ids, vec![7]);
        assert!(recognizer
            .feed_frame(frame(&[(0.2, 0.5), (0.1, 0.5), (0.0, 0.5)]), 48)
            .is_empty());
    }

    #[test]
    fn diagonal_motion_waits_for_a_dominant_axis() {
        let mut recognizer = GestureRecognizer::default();
        recognizer.configure(
            vec![
                registration(1, 2, GestureDirection::Left),
                registration(2, 2, GestureDirection::Up),
            ],
            350,
            1.5,
        );
        recognizer.feed_frame(frame(&[(0.6, 0.6), (0.4, 0.4)]), 0);
        assert!(recognizer
            .feed_frame(frame(&[(0.55, 0.55), (0.35, 0.35)]), 16)
            .is_empty());
        let event = recognizer
            .feed_frame(frame(&[(0.45, 0.57), (0.25, 0.37)]), 32)
            .remove(0);
        assert_eq!(event.direction, Some(GestureDirection::Left));
    }

    #[test]
    fn release_and_idle_timeout_cancel_incomplete_swipes() {
        let mut recognizer = GestureRecognizer::default();
        recognizer.configure(vec![registration(1, 4, GestureDirection::Right)], 100, 1.25);
        recognizer.feed_frame(frame(&[(0.2, 0.5), (0.3, 0.5), (0.4, 0.5), (0.5, 0.5)]), 0);
        assert_eq!(
            recognizer.feed_frame(frame(&[]), 20)[0].phase,
            GesturePhase::Cancelled
        );

        recognizer.feed_frame(frame(&[(0.2, 0.5), (0.3, 0.5), (0.4, 0.5), (0.5, 0.5)]), 30);
        assert_eq!(recognizer.tick(130).unwrap().phase, GesturePhase::Cancelled);
        assert!(recognizer.tick(500).is_none());
    }

    #[test]
    fn changing_finger_count_cancels_then_starts_the_matching_profile() {
        let mut recognizer = GestureRecognizer::default();
        recognizer.configure(
            vec![
                registration(2, 2, GestureDirection::Left),
                registration(3, 3, GestureDirection::Right),
            ],
            350,
            1.25,
        );
        recognizer.feed_frame(frame(&[(0.4, 0.5), (0.6, 0.5)]), 0);
        let events = recognizer.feed_frame(frame(&[(0.3, 0.5), (0.5, 0.5), (0.7, 0.5)]), 16);
        assert_eq!(
            events.iter().map(|event| event.phase).collect::<Vec<_>>(),
            vec![GesturePhase::Cancelled, GesturePhase::Started]
        );
    }
}
