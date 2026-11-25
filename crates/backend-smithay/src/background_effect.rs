//! The desktop shown soft behind parts of a surface whose own program
//! asks for it (**ext-background-effect-v1**).
//!
//! A layer rule of the configuration makes what lies behind a whole
//! surface soft, by the surface's name (`desktop.layer_rules`, `blur`).
//! That is right for a panel and wrong for a surface that covers a
//! display and draws a few see-through grounds on it (the zones of the
//! desktop): there the program knows where its grounds are, and says so.
//! This is the protocol for that: a region of the surface, set with the
//! surface's other state and taken up with its next picture. What lies
//! behind the region is drawn as behind a surface with the rule: the
//! display's own soft picture (`soft.rs`), the part of it that is there.
//!
//! - It is honoured for the surfaces of shells (layer surfaces). A window
//!   that asks is not refused, and gets nothing: windows are made
//!   see-through and soft by the window manager's own rules.
//! - A region is rectangles. Round corners come as rows of them, which
//!   the program makes; how soft the picture is, is the compositor's.
//! - A surface with such a region is no part of the soft picture itself,
//!   as one with the rule is not.

use crate::state::State;
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{
    get_region_attributes, with_states, Cacheable, RectangleKind, RegionAttributes,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// The most rectangles a surface's region is taken as: enough for some
/// dozens of grounds with round corners, and an end to what a program
/// can have drawn each frame.
pub const MOST_PARTS: usize = 1024;

/// The parts of a surface the desktop is soft behind, in the surface's
/// own logical pixels: set with its other state, current with its
/// picture.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BlurParts(pub Vec<Rectangle<i32, Logical>>);

impl Cacheable for BlurParts {
    fn commit(&mut self, _: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _: &DisplayHandle) {
        *into = self;
    }
}

/// A surface has an object of this protocol (it may have one).
#[derive(Default)]
struct Taken(AtomicBool);

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
}

/// A region as rectangles that do not overlap what was taken away: each
/// rectangle added as it comes, each one taken away cut out of those
/// before it. Not more than [`MOST_PARTS`] (what comes after is left
/// out), and none of no size.
pub fn parts_of(region: &RegionAttributes) -> Vec<Rectangle<i32, Logical>> {
    let mut parts: Vec<Rectangle<i32, Logical>> = Vec::new();
    for (kind, rect) in &region.rects {
        if rect.size.w <= 0 || rect.size.h <= 0 {
            continue;
        }
        match kind {
            RectangleKind::Add => {
                if parts.len() < MOST_PARTS {
                    parts.push(*rect);
                }
            }
            RectangleKind::Subtract => {
                parts = parts
                    .into_iter()
                    .flat_map(|part| part.subtract_rect(*rect))
                    .take(MOST_PARTS)
                    .collect();
            }
        }
    }
    parts
}

/// The parts as far as they lie on a surface of this size (the protocol
/// has them cut to it); what lies beside it is left out.
pub fn parts_within(
    parts: &[Rectangle<i32, Logical>],
    size: (i32, i32),
) -> Vec<Rectangle<i32, Logical>> {
    let whole = Rectangle::from_size((size.0.max(0), size.1.max(0)).into());
    parts
        .iter()
        .filter_map(|part| part.intersection(whole))
        .filter(|part| part.size.w > 0 && part.size.h > 0)
        .collect()
}

/// The parts of a surface the desktop is soft behind now.
pub fn blur_parts(surface: &WlSurface) -> Vec<Rectangle<i32, Logical>> {
    with_states(surface, |states| {
        states.cached_state.get::<BlurParts>().current().0.clone()
    })
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        // What can be had: said at once, as the protocol has it.
        init.init(resource, ())
            .capabilities(ext_background_effect_manager_v1::Capability::Blur);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        manager: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } =
            request
        {
            let taken = with_states(&surface, |states| {
                states.data_map.insert_if_missing_threadsafe(Taken::default);
                states
                    .data_map
                    .get::<Taken>()
                    .is_some_and(|taken| taken.0.swap(true, Ordering::SeqCst))
            });
            if taken {
                manager.post_error(
                    ext_background_effect_manager_v1::Error::BackgroundEffectExists,
                    "the surface has a background effect object already",
                );
                return;
            }
            init.init(id, surface.downgrade());
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, Weak<WlSurface>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        effect: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        surface: &Weak<WlSurface>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let surface = surface.upgrade().ok();
        // What is set is the surface's from its next commit on.
        let set = |surface: &WlSurface, parts: Vec<Rectangle<i32, Logical>>| {
            with_states(surface, |states| {
                *states.cached_state.get::<BlurParts>().pending() = BlurParts(parts);
            });
        };
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                let Some(surface) = surface else {
                    effect.post_error(
                        ext_background_effect_surface_v1::Error::SurfaceDestroyed,
                        "the surface is gone",
                    );
                    return;
                };
                let parts = region
                    .map(|region| parts_of(&get_region_attributes(&region)))
                    .unwrap_or_default();
                set(&surface, parts);
            }
            // The object is let go: nothing soft from the next commit
            // on, and the surface may be given another.
            ext_background_effect_surface_v1::Request::Destroy => {
                if let Some(surface) = surface {
                    set(&surface, Vec::new());
                    with_states(&surface, |states| {
                        if let Some(taken) = states.data_map.get::<Taken>() {
                            taken.0.store(false, Ordering::SeqCst);
                        }
                    });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (width, height).into())
    }

    fn area(parts: &[Rectangle<i32, Logical>]) -> i64 {
        parts
            .iter()
            .map(|part| i64::from(part.size.w) * i64::from(part.size.h))
            .sum()
    }

    #[test]
    fn a_region_is_its_rectangles_without_what_was_taken_away() {
        let region = RegionAttributes {
            rects: vec![
                (RectangleKind::Add, rect(0, 0, 100, 100)),
                (RectangleKind::Add, rect(200, 0, 50, 50)),
                // A hole in the first; the second is not touched.
                (RectangleKind::Subtract, rect(25, 25, 50, 50)),
                // Nothing at all.
                (RectangleKind::Add, rect(10, 10, 0, 40)),
            ],
        };
        let parts = parts_of(&region);
        assert_eq!(area(&parts), 100 * 100 - 50 * 50 + 50 * 50);
        assert!(parts.contains(&rect(200, 0, 50, 50)));
        // Nothing of the hole is left in any part.
        assert!(parts
            .iter()
            .all(|part| part.intersection(rect(25, 25, 50, 50)).is_none()));
        // What is added after something was taken away is all there.
        let again = RegionAttributes {
            rects: vec![
                (RectangleKind::Add, rect(0, 0, 10, 10)),
                (RectangleKind::Subtract, rect(0, 0, 10, 10)),
                (RectangleKind::Add, rect(2, 2, 4, 4)),
            ],
        };
        assert_eq!(parts_of(&again), [rect(2, 2, 4, 4)]);
        assert!(parts_of(&RegionAttributes { rects: Vec::new() }).is_empty());
    }

    #[test]
    fn a_region_is_not_more_rectangles_than_are_drawn() {
        let region = RegionAttributes {
            rects: (0..5000)
                .map(|row| (RectangleKind::Add, rect(0, row, 10, 1)))
                .collect(),
        };
        assert_eq!(parts_of(&region).len(), MOST_PARTS);
    }

    #[test]
    fn parts_are_cut_to_the_surface_and_those_beside_it_left_out() {
        let parts = [
            rect(10, 10, 100, 50),
            rect(-20, -20, 40, 40),
            rect(390, 290, 100, 100),
            rect(500, 10, 20, 20),
        ];
        assert_eq!(
            parts_within(&parts, (400, 300)),
            [
                rect(10, 10, 100, 50),
                rect(0, 0, 20, 20),
                rect(390, 290, 10, 10)
            ]
        );
        assert!(parts_within(&parts, (0, 0)).is_empty());
    }
}
