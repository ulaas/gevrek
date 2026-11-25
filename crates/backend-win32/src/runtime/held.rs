//! Windows that open while the mouse button is held.
//!
//! A tab pulled out of a browser's window becomes a window of its own
//! while the button is still down, and its program moves it along under
//! the pointer until the button is let go. Windows tells of no drag for
//! it (the program moves the window itself), so gevrek took it for a
//! window that had just opened, gave it a tile at once, and the two moved
//! it against each other until the button was released.
//!
//! So a window that opens while the button is down is not told of to the
//! engine until the button is up. If the pointer is then on it and it has
//! the keyboard, it was carried there by hand: it is told of as opened and
//! as dropped, and is tiled on the display it was dropped on, as any
//! window that is dragged to another display.

use crate::dwm::{WindowEvent, WindowHandle};

/// The windows whose opening is held back, in the order they opened.
#[derive(Debug, Default)]
pub struct Held {
    windows: Vec<WindowHandle>,
}

impl Held {
    /// Whether an opening is held back (the loop then looks at the button
    /// every pass: its release is announced by nothing).
    pub fn any(&self) -> bool {
        !self.windows.is_empty()
    }

    /// One pass's events as the engine is to hear them.
    ///
    /// While the button is down, the openings of windows the engine does
    /// not have yet (`is_new`) are taken out and kept. When it is up, the
    /// kept ones are told first, each followed by a drag's start and end
    /// if it was carried by hand (`carried`). A window that is gone is no
    /// longer kept.
    pub fn pass(
        &mut self,
        events: Vec<WindowEvent>,
        button_down: bool,
        is_new: impl Fn(WindowHandle) -> bool,
        carried: impl Fn(WindowHandle) -> bool,
    ) -> Vec<WindowEvent> {
        for event in &events {
            if let WindowEvent::Destroyed(gone) = event {
                self.windows.retain(|held| held != gone);
            }
        }
        if button_down {
            let mut passed = Vec::with_capacity(events.len());
            // (An opening told before the same window's end, in one pass,
            // is of a window that is gone by now.)
            let mut gone_later: Vec<WindowHandle> = Vec::new();
            for event in events.into_iter().rev() {
                match event {
                    WindowEvent::Destroyed(gone) => {
                        gone_later.push(gone);
                        passed.push(event);
                    }
                    WindowEvent::Created(opened) if is_new(opened) => {
                        if !gone_later.contains(&opened) && !self.windows.contains(&opened) {
                            self.windows.push(opened);
                        }
                    }
                    other => passed.push(other),
                }
            }
            passed.reverse();
            return passed;
        }
        if self.windows.is_empty() {
            return events;
        }
        let mut told = Vec::with_capacity(events.len() + self.windows.len() * 3);
        for window in std::mem::take(&mut self.windows) {
            told.push(WindowEvent::Created(window));
            if carried(window) {
                told.push(WindowEvent::DragStarted(window));
                told.push(WindowEvent::DragEnded(window));
            }
        }
        told.extend(events);
        told
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowHandle = WindowHandle(1);
    const B: WindowHandle = WindowHandle(2);

    #[test]
    fn nothing_is_held_while_no_button_is_down() {
        let mut held = Held::default();
        let events = vec![WindowEvent::Created(A), WindowEvent::Focused(A)];
        assert_eq!(held.pass(events.clone(), false, |_| true, |_| true), events);
        assert!(!held.any());
    }

    #[test]
    fn a_window_that_opens_under_a_held_button_waits_for_its_release() {
        let mut held = Held::default();
        let passed = held.pass(
            vec![
                WindowEvent::Focused(A),
                WindowEvent::Created(A),
                WindowEvent::Moved(A, 5, 5),
            ],
            true,
            |_| true,
            |_| true,
        );
        assert_eq!(
            passed,
            vec![WindowEvent::Focused(A), WindowEvent::Moved(A, 5, 5)]
        );
        assert!(held.any());
        // Told again while it waits: still one opening.
        assert!(held
            .pass(vec![WindowEvent::Created(A)], true, |_| true, |_| true)
            .is_empty());

        // The button is up and the window was carried: opened, then dropped.
        let told = held.pass(vec![WindowEvent::Focused(B)], false, |_| true, |_| true);
        assert_eq!(
            told,
            vec![
                WindowEvent::Created(A),
                WindowEvent::DragStarted(A),
                WindowEvent::DragEnded(A),
                WindowEvent::Focused(B),
            ]
        );
        assert!(!held.any());
    }

    #[test]
    fn a_window_that_only_opened_meanwhile_is_told_of_as_opened() {
        let mut held = Held::default();
        held.pass(vec![WindowEvent::Created(A)], true, |_| true, |_| false);
        assert_eq!(
            held.pass(Vec::new(), false, |_| true, |_| false),
            vec![WindowEvent::Created(A)]
        );
    }

    #[test]
    fn a_window_the_engine_has_is_not_held() {
        let mut held = Held::default();
        // (Shown again with its workspace: Windows tells of that as of an opening.)
        let events = vec![WindowEvent::Created(A)];
        assert_eq!(held.pass(events.clone(), true, |_| false, |_| true), events);
        assert!(!held.any());
    }

    #[test]
    fn a_window_that_closed_while_it_waited_is_forgotten() {
        let mut held = Held::default();
        held.pass(vec![WindowEvent::Created(A)], true, |_| true, |_| true);
        let passed = held.pass(vec![WindowEvent::Destroyed(A)], true, |_| true, |_| true);
        assert_eq!(passed, vec![WindowEvent::Destroyed(A)]);
        assert!(!held.any());
        // Opened and gone within one pass.
        let passed = held.pass(
            vec![WindowEvent::Created(B), WindowEvent::Destroyed(B)],
            true,
            |_| true,
            |_| true,
        );
        assert_eq!(passed, vec![WindowEvent::Destroyed(B)]);
        assert!(!held.any());
    }
}
