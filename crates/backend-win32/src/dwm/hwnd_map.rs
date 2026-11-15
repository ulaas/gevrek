//! Window ids for HWNDs (docs/PHASE2_PLAN.md, decision D2, stage 2).
//!
//! Windows reuses HWND values: after a window is destroyed, a new window can
//! get the same value. The engine keys everything it knows about a window by
//! its `WindowId`, so the id must not be the bare HWND, or the new window
//! would inherit whatever was still recorded for the old one.
//!
//! An id is the HWND value in its low 32 bits (an HWND has 32 significant
//! bits, also in a 64-bit process) and a generation above them. The
//! generation of an HWND value goes up each time a window with that value is
//! destroyed. So:
//!
//! - the first window with an HWND value has an id equal to that value, which
//!   keeps `handle:<hwnd>` selectors and logs readable;
//! - a later window with the same value has a different id;
//! - getting the HWND back from an id needs no lookup.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const HWND_BITS: u32 = 32;
const HWND_MASK: isize = 0xFFFF_FFFF;

fn generations() -> &'static Mutex<HashMap<u32, u32>> {
    static GENERATIONS: OnceLock<Mutex<HashMap<u32, u32>>> = OnceLock::new();
    GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn low(hwnd: isize) -> u32 {
    (hwnd & HWND_MASK) as u32
}

/// The id of the window that currently has this HWND value.
pub fn id_for(hwnd: isize) -> isize {
    let generation = generations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&low(hwnd))
        .copied()
        .unwrap_or(0);
    ((generation as isize) << HWND_BITS) | (low(hwnd) as isize)
}

/// The HWND value an id stands for (sign-extended, as Windows hands it out).
pub fn hwnd_of(id: isize) -> isize {
    low(id) as i32 as isize
}

/// The window with this HWND value was destroyed: the next window that gets
/// the value is a different window.
pub fn retire(hwnd: isize) {
    let mut generations = generations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generation = generations.entry(low(hwnd)).or_insert(0);
    // Stay within the positive range of the id.
    *generation = (*generation + 1) & 0x7FFF_FFFF;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_window_with_a_handle_value_keeps_it_as_its_id() {
        let hwnd = 0x00A1_0001;
        assert_eq!(id_for(hwnd), hwnd);
        assert_eq!(hwnd_of(id_for(hwnd)), hwnd);
    }

    #[test]
    fn a_reused_handle_value_is_a_different_window() {
        let hwnd = 0x00A1_0002;
        let first = id_for(hwnd);
        retire(hwnd);
        let second = id_for(hwnd);
        assert_ne!(first, second);
        assert_eq!(id_for(hwnd), second, "stable until the next destroy");
        // Both still name the same HWND value for Win32 calls.
        assert_eq!((hwnd_of(first), hwnd_of(second)), (hwnd, hwnd));
        retire(hwnd);
        assert_ne!(id_for(hwnd), second);
        // Other handle values are not affected.
        assert_eq!(id_for(0x00A1_0003), 0x00A1_0003);
    }

    #[test]
    fn a_sign_extended_handle_round_trips() {
        let hwnd = 0xFFFF_FFFF_8001_0004_u64 as isize;
        assert_eq!(hwnd_of(id_for(hwnd)), hwnd);
    }
}
