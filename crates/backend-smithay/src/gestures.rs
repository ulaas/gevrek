//! Touchpad swipes (`desktop.gestures`). libinput recognises the swipe and
//! reports how many fingers and how far; gevrek decides the direction and
//! runs the command of the matching profile when the fingers lift.
//!
//! A sideways swipe with `scroll_fingers` fingers over a display that shows
//! a strip (the scrolling layout) is not a command at the end: the view
//! goes along with the fingers while they move, and stays where they left
//! it. So does a wheel or two fingers with the modifier of the mouse held.
//!
//! A swipe whose profile's command goes to the workspace before or after
//! pulls that workspace into view as the fingers go; when they lift, the
//! change is made from where they left it, or everything goes back.
//!
//! On Windows gevrek reads raw contacts and recognises swipes itself; here
//! `distance_percent` is taken as a percentage of the smaller side of the
//! output under the pointer (libinput reports distances in the units the
//! pointer moves in).

use gevrek_host::wm::commands::Commands as _;
use crate::state::State;
use gevrek_engine::executor::RuntimeEventKind;
use gevrek_engine::overlay_data::GestureProgressData;
use gevrek_model::settings::GestureDirection;

/// A swipe in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swipe {
    pub fingers: u32,
    pub dx: f64,
    pub dy: f64,
    /// Whether it moves the view along a strip.
    pub strip: Strip,
    /// The part of a pixel the view has not been moved by yet.
    pub rest: f64,
    /// Whether it was looked at already for a workspace it could pull.
    pub asked: bool,
}

/// A swipe that pulls the workspace beside into view.
#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    /// The direction of the fingers that pulls it.
    pub direction: GestureDirection,
    /// What is pulled (`gone` as the fingers have it now).
    pub follow: crate::desktop::FollowWorkspace,
    /// How far the fingers go for the whole way.
    pub span: f64,
}

/// The part of a swipe's reach (the smaller side of the display) after
/// which a pulled workspace is in view altogether.
const PULL_SPAN: f64 = 0.6;

/// What a swipe is to the strip under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strip {
    /// Not for it: the fingers are not as many, or no strip is there.
    No,
    /// It could be; the fingers have not gone far enough to say which way.
    Perhaps,
    /// The view follows the fingers.
    Follows,
}

/// How far the fingers go before a swipe is taken as sideways or not.
const DECIDED_AFTER: f64 = 8.0;
/// Pixels of the strip for one unit the fingers move (libinput's units
/// are those the pointer moves in; three fingers over the width of a
/// touchpad go about two columns).
const STRIP_PER_UNIT: f64 = 2.5;
/// Pixels of the strip for one unit of a wheel (a notch is fifteen).
const STRIP_PER_WHEEL_UNIT: f64 = 12.0;
/// Pixels of the strip for one unit of two fingers scrolling.
const STRIP_PER_FINGER_UNIT: f64 = 2.0;

/// The direction of a movement, once one axis clearly leads: its distance
/// is at least `axis_lock_ratio` times the other's.
pub fn direction(dx: f64, dy: f64, axis_lock_ratio: f32) -> Option<GestureDirection> {
    let ratio = f64::from(axis_lock_ratio.max(1.0));
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax == 0.0 && ay == 0.0 {
        None
    } else if ax >= ay * ratio {
        Some(if dx < 0.0 {
            GestureDirection::Left
        } else {
            GestureDirection::Right
        })
    } else if ay >= ax * ratio {
        Some(if dy < 0.0 {
            GestureDirection::Up
        } else {
            GestureDirection::Down
        })
    } else {
        None
    }
}

impl State {
    /// Whether a swipe with this many fingers is one the configuration
    /// gives a command to (then it is not passed on to a program).
    pub fn swipe_is_ours(&self, fingers: u32) -> bool {
        let gestures = &self.config.desktop.gestures;
        gestures.enabled
            && (gestures
                .profiles
                .iter()
                .any(|profile| profile.enabled && u32::from(profile.fingers) == fingers)
                || self.swipe_is_the_strips(fingers))
    }

    /// Whether a swipe with this many fingers would move the view of the
    /// strip under the pointer.
    fn swipe_is_the_strips(&self, fingers: u32) -> bool {
        let gestures = &self.config.desktop.gestures;
        gestures.enabled
            && gestures.scroll_fingers != 0
            && u32::from(gestures.scroll_fingers) == fingers
            && !self.paused
            && self.strip_under_pointer()
    }

    /// Whether the display the pointer is on shows a strip whose view can
    /// be moved.
    pub fn strip_under_pointer(&self) -> bool {
        self.core
            .executor
            .strip_under_pointer(&self.desktop, &self.core.monitor_manager)
    }

    /// Move the view of the strip under the pointer by so many pixels:
    /// at once (`follow`: fingers are leading it) or with the layout's
    /// transition (a wheel's step). Returns whether it moved.
    pub fn scroll_strip(&mut self, pixels: i32, follow: bool) -> bool {
        if pixels == 0 {
            return false;
        }
        self.desktop.follow = follow;
        let moved = self.core.executor.scroll_view(
            &mut self.desktop,
            &mut self.core.monitor_manager,
            pixels,
        );
        if moved {
            self.settle();
        }
        self.desktop.follow = false;
        moved
    }

    /// A wheel turned or two fingers scrolled while the modifier of the
    /// mouse was held: over a strip that moves its view (a wheel in
    /// steps, fingers as they go) and the program under the pointer hears
    /// nothing of it. Returns whether it was taken.
    pub fn strip_wheel(&mut self, horizontal: f64, vertical: f64, fingers: bool) -> bool {
        if self.paused || !self.strip_under_pointer() {
            return false;
        }
        let amount = if horizontal.abs() >= vertical.abs() {
            horizontal
        } else {
            vertical
        };
        if amount == 0.0 {
            return false;
        }
        let per_unit = if fingers {
            STRIP_PER_FINGER_UNIT
        } else {
            STRIP_PER_WHEEL_UNIT
        };
        let pixels = (amount * per_unit).round() as i32;
        self.scroll_strip(
            if pixels == 0 {
                amount.signum() as i32
            } else {
                pixels
            },
            fingers,
        );
        true
    }

    /// Fingers went down for a swipe.
    pub fn swipe_begin(&mut self, fingers: u32) {
        let strip = if self.swipe_is_the_strips(fingers) {
            Strip::Perhaps
        } else {
            Strip::No
        };
        self.swipe = Some(Swipe {
            fingers,
            dx: 0.0,
            dy: 0.0,
            strip,
            rest: 0.0,
            asked: false,
        });
        self.swipe_pull = None;
        self.report_swipe(RuntimeEventKind::GestureStarted, "started");
    }

    /// The fingers moved.
    pub fn swipe_update(&mut self, dx: f64, dy: f64) {
        let ratio = f64::from(self.config.desktop.gestures.axis_lock_ratio.max(1.0));
        let Some(swipe) = self.swipe.as_mut() else {
            return;
        };
        swipe.dx += dx;
        swipe.dy += dy;
        let mut by = 0;
        if swipe.strip == Strip::Perhaps && swipe.dx.abs().max(swipe.dy.abs()) >= DECIDED_AFTER {
            if swipe.dx.abs() >= swipe.dy.abs() * ratio {
                // Sideways: the view takes up all the way gone so far.
                swipe.strip = Strip::Follows;
                swipe.rest = -(swipe.dx - dx) * STRIP_PER_UNIT;
            } else {
                swipe.strip = Strip::No;
            }
        }
        if swipe.strip == Strip::Follows {
            // The strip goes with the fingers: fingers to the left bring
            // what is on the right into view.
            swipe.rest -= dx * STRIP_PER_UNIT;
            by = swipe.rest.trunc() as i32;
            swipe.rest -= f64::from(by);
        }
        let swipe = *swipe;
        self.scroll_strip(by, true);
        self.pull_workspace(swipe);
        self.report_swipe(RuntimeEventKind::GestureUpdated, "updated");
    }

    /// The fingers moved: a swipe that would change to the workspace
    /// before or after pulls it into view by as much as they have gone.
    fn pull_workspace(&mut self, swipe: Swipe) {
        use gevrek_engine::animation_scheduler::TransitionScope;
        if swipe.strip == Strip::Follows {
            return;
        }
        let settings = &self.config.desktop.gestures;
        if self.swipe_pull.is_none() && !swipe.asked {
            let Some(direction) = direction(swipe.dx, swipe.dy, settings.axis_lock_ratio)
                .filter(|_| swipe.dx.abs().max(swipe.dy.abs()) >= DECIDED_AFTER)
            else {
                return;
            };
            if let Some(swipe) = self.swipe.as_mut() {
                swipe.asked = true;
            }
            let selector = settings
                .profiles
                .iter()
                .find(|profile| {
                    profile.enabled
                        && u32::from(profile.fingers) == swipe.fingers
                        && profile.direction == direction
                })
                .and_then(|profile| match &profile.command {
                    gevrek_model::command::DispatchCommand::SwitchWorkspaceSelector {
                        selector,
                    } => Some(selector.clone()),
                    _ => None,
                });
            let able = settings.enabled
                && !self.paused
                && self.desktop.animations.moves(TransitionScope::Workspace);
            let Some(selector) = selector.filter(|_| able) else {
                return;
            };
            let Some(beside) = self.core.executor.workspace_beside(
                &mut self.desktop,
                &mut self.core.monitor_manager,
                &selector,
            ) else {
                return;
            };
            let apart = match direction {
                GestureDirection::Left | GestureDirection::Right => {
                    (beside.direction * beside.area.width, 0)
                }
                GestureDirection::Up | GestureDirection::Down => {
                    (0, beside.direction * beside.area.height)
                }
            };
            self.swipe_pull = Some(Pull {
                direction,
                follow: crate::desktop::FollowWorkspace {
                    going: beside.going,
                    coming: beside.coming,
                    apart,
                    gone: 0.0,
                },
                span: (self.swipe_reach() * PULL_SPAN).max(1.0),
            });
        }
        let Some(pull) = self.swipe_pull.as_mut() else {
            return;
        };
        let along = match pull.direction {
            GestureDirection::Left => -swipe.dx,
            GestureDirection::Right => swipe.dx,
            GestureDirection::Up => -swipe.dy,
            GestureDirection::Down => swipe.dy,
        };
        pull.follow.gone = (along / pull.span).clamp(0.0, 1.0) as f32;
        self.desktop.follow_workspace(pull.follow.clone());
    }

    /// The fingers lifted (or the swipe was cancelled): a swipe that went
    /// far enough in one direction runs the command of its profile.
    /// Returns whether a command ran.
    pub fn swipe_end(&mut self, cancelled: bool) -> bool {
        let Some(swipe) = self.swipe else {
            return false;
        };
        if swipe.strip == Strip::Follows {
            // The view stays where the fingers left it; nothing else runs.
            self.report_swipe(RuntimeEventKind::GestureCompleted, "completed");
            self.swipe = None;
            return true;
        }
        let settings = self.config.desktop.gestures.clone();
        let reach = self.swipe_reach();
        let direction = direction(swipe.dx, swipe.dy, settings.axis_lock_ratio);
        let command = (!cancelled && settings.enabled && !self.paused)
            .then_some(direction)
            .flatten()
            .and_then(|direction| {
                let travelled = match direction {
                    GestureDirection::Left | GestureDirection::Right => swipe.dx.abs(),
                    GestureDirection::Up | GestureDirection::Down => swipe.dy.abs(),
                };
                settings
                    .profiles
                    .iter()
                    .find(|profile| {
                        profile.enabled
                            && u32::from(profile.fingers) == swipe.fingers
                            && profile.direction == direction
                            && travelled >= reach * f64::from(profile.distance_percent) / 100.0
                    })
                    .map(|profile| profile.command.clone())
            });
        let ran = command.is_some();
        self.report_swipe(
            if ran {
                RuntimeEventKind::GestureCompleted
            } else {
                RuntimeEventKind::GestureCancelled
            },
            if ran { "completed" } else { "cancelled" },
        );
        self.swipe = None;
        self.swipe_pull = None;
        if let Some(command) = command {
            if let Err(error) = self.run_command(command) {
                log::error!("Gesture command failed: {error}");
            }
        }
        // What was pulled and is not on its way by the command now (the
        // fingers did not go far enough, or went another way in the end)
        // goes back.
        self.desktop.follow_workspace_back();
        ran
    }

    /// What 100 percent of a swipe is: the smaller side of the output the
    /// pointer is on.
    fn swipe_reach(&self) -> f64 {
        let output = gevrek_engine::Desktop::output_at_cursor(&self.desktop)
            .and_then(|id| self.desktop.output_rectangle(id));
        output.map_or(1000.0, |rect| f64::from(rect.width.min(rect.height)))
    }

    /// Tell listeners and the state snapshot how the swipe is going.
    fn report_swipe(&mut self, kind: RuntimeEventKind, phase: &str) {
        let Some(swipe) = self.swipe else {
            return;
        };
        let reach = self.swipe_reach();
        let direction = direction(
            swipe.dx,
            swipe.dy,
            self.config.desktop.gestures.axis_lock_ratio,
        );
        let progress = (swipe.dx.abs().max(swipe.dy.abs()) / reach) as f32;
        self.gesture_progress = Some(GestureProgressData {
            phase: phase.to_string(),
            fingers: swipe.fingers as u8,
            direction: direction.map(|direction| direction.config_name().to_string()),
            progress,
            delta_x: swipe.dx as f32,
            delta_y: swipe.dy as f32,
        });
        self.emit(
            kind,
            None,
            vec![
                ("fingers".to_string(), swipe.fingers.to_string()),
                (
                    "direction".to_string(),
                    direction
                        .map_or("pending", |direction| direction.config_name())
                        .to_string(),
                ),
                ("progress".to_string(), format!("{progress:.4}")),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direction_needs_one_axis_to_lead_clearly() {
        assert_eq!(direction(-100.0, 5.0, 1.25), Some(GestureDirection::Left));
        assert_eq!(direction(100.0, -20.0, 1.25), Some(GestureDirection::Right));
        assert_eq!(direction(3.0, -80.0, 1.25), Some(GestureDirection::Up));
        assert_eq!(direction(0.0, 80.0, 1.25), Some(GestureDirection::Down));
        assert_eq!(direction(100.0, 90.0, 1.25), None, "a diagonal is neither");
        assert_eq!(direction(0.0, 0.0, 1.25), None);
    }
}
