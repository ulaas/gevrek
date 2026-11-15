//! Collapses one wake's worth of window events before the executor sees them.
//!
//! Title-spamming apps (browsers, terminals) and drags produce many events per
//! window per frame. Only the newest geometry and metadata of a window matter,
//! so older ones are dropped, as long as no lifecycle, drag, or state event of
//! the same window lies between them (those change how later events are
//! interpreted, so collapsing across them would change behaviour). Geometry and
//! metadata events of a window that is destroyed later in the batch are
//! dropped entirely: they would only query a dead window.

use crate::events::WindowEvent;
use crate::WindowHandle;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Collapsible {
    Moved,
    Resized,
    Metadata,
}

fn collapsible(event: &WindowEvent) -> Option<(WindowHandle, Collapsible)> {
    match event {
        WindowEvent::Moved(handle, _, _) => Some((*handle, Collapsible::Moved)),
        WindowEvent::Resized(handle, _, _) => Some((*handle, Collapsible::Resized)),
        WindowEvent::MetadataChanged(handle) => Some((*handle, Collapsible::Metadata)),
        _ => None,
    }
}

fn event_handle(event: &WindowEvent) -> WindowHandle {
    match event {
        WindowEvent::Created(handle)
        | WindowEvent::Destroyed(handle)
        | WindowEvent::Focused(handle)
        | WindowEvent::Moved(handle, _, _)
        | WindowEvent::Resized(handle, _, _)
        | WindowEvent::DragStarted(handle)
        | WindowEvent::DragEnded(handle)
        | WindowEvent::Minimized(handle)
        | WindowEvent::Restored(handle)
        | WindowEvent::MetadataChanged(handle)
        | WindowEvent::Unmapped(handle) => *handle,
    }
}

/// Collapse a batch of events, keeping the relative order of what remains.
pub fn coalesce_window_events(events: Vec<WindowEvent>) -> Vec<WindowEvent> {
    if events.len() < 2 {
        return events;
    }
    // Walk backwards: an event is redundant when a newer one of the same kind
    // for the same window survives with no barrier in between.
    let mut newer: HashSet<(WindowHandle, Collapsible)> = HashSet::new();
    let mut destroyed_later: HashSet<WindowHandle> = HashSet::new();
    let mut keep = vec![true; events.len()];
    for (index, event) in events.iter().enumerate().rev() {
        if let Some(key) = collapsible(event) {
            if destroyed_later.contains(&key.0) || !newer.insert(key) {
                keep[index] = false;
            }
            continue;
        }
        // Any other event of the window is a barrier for collapsing.
        let handle = event_handle(event);
        newer.retain(|(other, _)| *other != handle);
        match event {
            WindowEvent::Destroyed(_) => {
                destroyed_later.insert(handle);
            }
            // Events before a Created belong to an earlier use of the handle.
            WindowEvent::Created(_) => {
                destroyed_later.remove(&handle);
            }
            _ => {}
        }
    }
    let mut keep = keep.into_iter();
    events
        .into_iter()
        .filter(|_| keep.next().unwrap_or(true))
        .collect()
}

/// Focus effects (borders, inactive opacity, dimming) depend on which window
/// is focused and which windows exist. Geometry and title changes alone never
/// change them, so a batch of only those does not re-run the effects.
pub fn affects_focus_effects(events: &[WindowEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            WindowEvent::Focused(_)
                | WindowEvent::Created(_)
                | WindowEvent::Destroyed(_)
                | WindowEvent::Minimized(_)
                | WindowEvent::Restored(_)
                | WindowEvent::DragEnded(_)
                | WindowEvent::Unmapped(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowHandle = WindowHandle(1);
    const B: WindowHandle = WindowHandle(2);

    #[test]
    fn a_title_storm_collapses_to_the_last_change_per_window() {
        let events = vec![
            WindowEvent::MetadataChanged(A),
            WindowEvent::MetadataChanged(B),
            WindowEvent::MetadataChanged(A),
            WindowEvent::MetadataChanged(A),
        ];
        assert_eq!(
            coalesce_window_events(events),
            vec![
                WindowEvent::MetadataChanged(B),
                WindowEvent::MetadataChanged(A)
            ]
        );
    }

    #[test]
    fn geometry_keeps_the_newest_move_and_resize() {
        let events = vec![
            WindowEvent::Moved(A, 1, 1),
            WindowEvent::Resized(A, 10, 10),
            WindowEvent::Moved(A, 2, 2),
            WindowEvent::Resized(A, 20, 20),
            WindowEvent::Moved(B, 5, 5),
        ];
        assert_eq!(
            coalesce_window_events(events),
            vec![
                WindowEvent::Moved(A, 2, 2),
                WindowEvent::Resized(A, 20, 20),
                WindowEvent::Moved(B, 5, 5),
            ]
        );
    }

    #[test]
    fn nothing_collapses_across_a_drag_boundary_of_the_same_window() {
        let events = vec![
            WindowEvent::Moved(A, 1, 1),
            WindowEvent::DragEnded(A),
            WindowEvent::Moved(A, 2, 2),
        ];
        assert_eq!(coalesce_window_events(events.clone()), events);
    }

    #[test]
    fn another_windows_events_are_not_a_barrier() {
        let events = vec![
            WindowEvent::Moved(A, 1, 1),
            WindowEvent::Focused(B),
            WindowEvent::Moved(A, 2, 2),
        ];
        assert_eq!(
            coalesce_window_events(events),
            vec![WindowEvent::Focused(B), WindowEvent::Moved(A, 2, 2)]
        );
    }

    #[test]
    fn updates_of_a_window_destroyed_later_are_dropped() {
        let events = vec![
            WindowEvent::Created(A),
            WindowEvent::MetadataChanged(A),
            WindowEvent::Moved(A, 1, 1),
            WindowEvent::Destroyed(A),
        ];
        assert_eq!(
            coalesce_window_events(events),
            vec![WindowEvent::Created(A), WindowEvent::Destroyed(A)]
        );
    }

    #[test]
    fn a_reused_handle_keeps_the_new_windows_updates() {
        let events = vec![
            WindowEvent::Destroyed(A),
            WindowEvent::Created(A),
            WindowEvent::MetadataChanged(A),
        ];
        assert_eq!(coalesce_window_events(events.clone()), events);
    }

    #[test]
    fn only_focus_relevant_batches_rerun_focus_effects() {
        assert!(!affects_focus_effects(&[
            WindowEvent::MetadataChanged(A),
            WindowEvent::Moved(A, 1, 1),
            WindowEvent::Resized(B, 2, 2),
        ]));
        assert!(affects_focus_effects(&[
            WindowEvent::MetadataChanged(A),
            WindowEvent::Focused(B),
        ]));
        assert!(!affects_focus_effects(&[]));
    }
}
