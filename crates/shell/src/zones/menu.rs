//! Menus that open from each other: which of them are open, which line
//! the pointer or the keys are on, and what follows from a movement, a
//! press or a key. Nothing here opens a surface or draws: what comes out
//! is how the menus stand afterwards and what was chosen, and
//! `surfaces` makes the open surfaces fit. So all of it is tried in tests.
//!
//! A line with an arrow leads to a menu of its own. It opens beside its
//! line when the pointer rests on the line for a moment, at once when the
//! line is pressed, and with the right arrow or Enter; the menu it opened
//! from stays. Only one way through the menus is open at a time: resting
//! on another line of a menu closes what had opened from it. Escape and
//! the left arrow go one menu back; a line that does something closes
//! them all.

use super::surfaces::{Does, Line};
use egui::{pos2, vec2, Rect};

/// How long the pointer rests on a line before the menu of that line
/// opens, or the one that was open from its menu closes: long enough to
/// cross a menu on the way to the one beside it without the way closing.
pub const REST: f64 = 0.22;

pub const LINE_HEIGHT: f32 = 28.0;
pub const GAP_HEIGHT: f32 = 9.0;
pub const MENU_WIDTH: f32 = 236.0;
/// The room around the lines of a menu, and between two of them.
const MARGIN: f32 = 6.0;
const BETWEEN: f32 = 2.0;

/// How large a menu of these lines is, in points.
pub fn menu_size(lines: &[Line]) -> egui::Vec2 {
    let high: f32 = lines
        .iter()
        .map(|line| {
            if *line == Line::Gap {
                GAP_HEIGHT
            } else {
                LINE_HEIGHT + BETWEEN
            }
        })
        .sum();
    vec2(MENU_WIDTH, 2.0 * MARGIN + high)
}

/// Where each line of a menu is, in the menu's own points: a rectangle
/// for what can be chosen, and for a gap the line that is drawn in it.
pub fn rows(lines: &[Line]) -> Vec<Rect> {
    let mut top = MARGIN;
    lines
        .iter()
        .map(|line| {
            let high = if *line == Line::Gap {
                GAP_HEIGHT - BETWEEN
            } else {
                LINE_HEIGHT
            };
            let row = Rect::from_min_size(pos2(MARGIN, top), vec2(MENU_WIDTH - 2.0 * MARGIN, high));
            top += high + BETWEEN;
            row
        })
        .collect()
}

/// A key that means something to a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    First,
    Last,
    /// Into the menu of the line (the right arrow).
    Into,
    /// Back to the menu this one opened from (the left arrow).
    Back,
    /// Choose the line, or go into its menu (Enter, Space).
    Choose,
    /// One menu back; from the first, all are closed.
    Escape,
}

/// One menu that is open.
#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    /// What tells this menu from every other that was ever open: its
    /// surface draws this one and no later menu at the same depth.
    pub serial: u64,
    pub lines: Vec<Line>,
    /// The line the pointer or the keys are on.
    pub at: Option<usize>,
    /// The line whose menu is open beside this one.
    pub open: Option<usize>,
}

impl Level {
    fn can_choose(&self, line: usize) -> bool {
        self.lines.get(line).is_some_and(|line| *line != Line::Gap)
    }

    fn leads_on(&self, line: usize) -> bool {
        matches!(self.lines.get(line), Some(Line::Sub { .. }))
    }

    /// The next line that can be chosen from `from`, going down or up
    /// and round at the ends; from nowhere, the first or the last.
    fn step(&self, from: Option<usize>, down: bool) -> Option<usize> {
        let count = self.lines.len();
        let order: Vec<usize> = match (from, down) {
            (None, true) => (0..count).collect(),
            (None, false) => (0..count).rev().collect(),
            (Some(from), true) => (1..=count).map(|by| (from + by) % count).collect(),
            (Some(from), false) => (1..=count)
                .map(|by| (from + count - by % count) % count)
                .collect(),
        };
        order.into_iter().find(|line| self.can_choose(*line))
    }
}

/// What the pointer waits to do, resting on a line.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Resting {
    level: usize,
    line: usize,
    since: f64,
}

/// The menus that are open, the first one first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chain {
    pub levels: Vec<Level>,
    resting: Option<Resting>,
    serials: u64,
}

impl Chain {
    /// One menu, open. `by_keys`: asked for by a key, so the keys are on
    /// its first line already.
    pub fn of(lines: Vec<Line>, by_keys: bool) -> Self {
        let mut chain = Chain::default();
        chain.push(lines, by_keys);
        chain
    }

    fn push(&mut self, lines: Vec<Line>, by_keys: bool) {
        self.serials += 1;
        let mut level = Level {
            serial: self.serials,
            lines,
            at: None,
            open: None,
        };
        if by_keys {
            level.at = level.step(None, true);
        }
        self.levels.push(level);
    }

    /// Which menu a surface draws, by what tells it from the others.
    pub fn level_of(&self, serial: u64) -> Option<usize> {
        self.levels.iter().position(|level| level.serial == serial)
    }

    /// Close every menu from `depth` on (0 closes them all).
    pub fn close_from(&mut self, depth: usize) {
        self.levels.truncate(depth);
        if let Some(last) = self.levels.last_mut() {
            last.open = None;
        }
        if self.resting.is_some_and(|resting| resting.level >= depth) {
            self.resting = None;
        }
    }

    /// Open the menu a line leads to, beside its own; whatever was open
    /// from that menu goes first.
    fn open(&mut self, level: usize, line: usize, by_keys: bool) {
        let Some(Line::Sub { lines, .. }) = self
            .levels
            .get(level)
            .and_then(|menu| menu.lines.get(line))
            .cloned()
        else {
            return;
        };
        self.close_from(level + 1);
        self.levels[level].open = Some(line);
        self.levels[level].at = Some(line);
        self.push(lines, by_keys);
    }

    /// The pointer is on a line of a menu (or on none of its lines). What
    /// follows comes after a moment: [`Chain::rested`].
    pub fn pointed(&mut self, level: usize, line: Option<usize>, now: f64) {
        let Some(menu) = self.levels.get_mut(level) else {
            return;
        };
        let Some(line) = line.filter(|line| menu.can_choose(*line)) else {
            // Off its lines: what was waited for is no longer.
            if self.resting.is_some_and(|resting| resting.level == level) {
                self.resting = None;
            }
            return;
        };
        menu.at = Some(line);
        // On the line whose menu is open: it stays, and nothing waits.
        if menu.open == Some(line) {
            self.resting = None;
            return;
        }
        // Something would change after a moment: its menu opens, or the
        // one that is open from here closes.
        let changes = menu.leads_on(line) || menu.open.is_some();
        match self.resting {
            Some(resting) if resting.level == level && resting.line == line => {}
            _ => {
                self.resting = changes.then_some(Resting {
                    level,
                    line,
                    since: now,
                })
            }
        }
    }

    /// How long until the pointer has rested long enough, in seconds;
    /// none while nothing waits.
    pub fn rests_in(&self, now: f64) -> Option<f64> {
        self.resting
            .map(|resting| (resting.since + REST - now).max(0.0))
    }

    /// Time has passed. Says whether the menus changed.
    pub fn rested(&mut self, now: f64) -> bool {
        let Some(resting) = self.resting.filter(|resting| now - resting.since >= REST) else {
            return false;
        };
        self.resting = None;
        let Some(menu) = self.levels.get(resting.level) else {
            return false;
        };
        if menu.leads_on(resting.line) {
            self.open(resting.level, resting.line, false);
        } else {
            self.close_from(resting.level + 1);
        }
        true
    }

    /// A line was pressed: what it does, if it does something (the menus
    /// are then to be closed, all of them). A line that leads on opens
    /// its menu at once, and nothing is chosen.
    pub fn pressed(&mut self, level: usize, line: usize) -> Option<Does> {
        self.resting = None;
        match self.levels.get(level)?.lines.get(line)?.clone() {
            Line::Choice { does, .. } => Some(does),
            Line::Sub { .. } => {
                // (Pressed again while it is open: it stays open.)
                if self.levels[level].open != Some(line) {
                    self.open(level, line, false);
                }
                None
            }
            Line::Gap => None,
        }
    }

    /// A key, for the menu that opened last. What was chosen, if a line
    /// that does something was.
    pub fn key(&mut self, key: Key) -> Option<Does> {
        self.resting = None;
        let depth = self.levels.len().checked_sub(1)?;
        let menu = &mut self.levels[depth];
        match key {
            Key::Down => menu.at = menu.step(menu.at, true),
            Key::Up => menu.at = menu.step(menu.at, false),
            Key::First => menu.at = menu.step(None, true),
            Key::Last => menu.at = menu.step(None, false),
            Key::Into => {
                if let Some(line) = menu.at.filter(|line| menu.leads_on(*line)) {
                    self.open(depth, line, true);
                }
            }
            Key::Back => {
                if depth > 0 {
                    self.close_from(depth);
                }
            }
            Key::Escape => self.close_from(depth),
            Key::Choose => {
                let line = menu.at?;
                if menu.leads_on(line) {
                    self.open(depth, line, true);
                } else if let Some(Line::Choice { does, .. }) = menu.lines.get(line) {
                    return Some(does.clone());
                }
            }
        }
        None
    }
}

/// The keys of a frame that are a menu's, taken out of what the surface
/// heard so that nothing else acts on them.
pub fn keys(input: &mut egui::InputState) -> Vec<Key> {
    let mut heard = Vec::new();
    for (key, means) in [
        (egui::Key::ArrowDown, Key::Down),
        (egui::Key::ArrowUp, Key::Up),
        (egui::Key::Home, Key::First),
        (egui::Key::End, Key::Last),
        (egui::Key::ArrowRight, Key::Into),
        (egui::Key::ArrowLeft, Key::Back),
        (egui::Key::Enter, Key::Choose),
        (egui::Key::Space, Key::Choose),
        (egui::Key::Escape, Key::Escape),
    ] {
        for _ in 0..input.count_and_consume_key(egui::Modifiers::NONE, key) {
            heard.push(means);
        }
    }
    heard
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::core::Command;

    fn does(name: &str) -> Does {
        Does::Command(Command::Search {
            typed: name.to_string(),
        })
    }

    fn choice(name: &str) -> Line {
        Line::Choice {
            label: name.to_string(),
            marked: false,
            does: does(name),
        }
    }

    fn sub(name: &str, lines: Vec<Line>) -> Line {
        Line::Sub {
            label: name.to_string(),
            lines,
        }
    }

    /// Rename / Shown ▸ (Grid, List, Order ▸ (Name, Date)) / gap / Look ▸ (Solid) / Away
    fn menu() -> Vec<Line> {
        vec![
            choice("Rename"),
            sub(
                "Shown",
                vec![
                    choice("Grid"),
                    choice("List"),
                    Line::Gap,
                    sub("Order", vec![choice("Name"), choice("Date")]),
                ],
            ),
            Line::Gap,
            sub("Look", vec![choice("Solid")]),
            choice("Away"),
        ]
    }

    fn open_lines(chain: &Chain) -> Vec<Option<usize>> {
        chain.levels.iter().map(|level| level.open).collect()
    }

    #[test]
    fn resting_on_a_line_with_an_arrow_opens_its_menu_and_the_first_stays() {
        let mut chain = Chain::of(menu(), false);
        chain.pointed(0, Some(1), 10.0);
        assert_eq!(chain.levels.len(), 1, "not at once");
        assert!(!chain.rested(10.1));
        assert!(chain
            .rests_in(10.1)
            .is_some_and(|left| left > 0.0 && left < REST));
        assert!(chain.rested(10.0 + REST));
        assert_eq!(chain.levels.len(), 2);
        assert_eq!(open_lines(&chain), [Some(1), None]);
        assert_eq!(chain.rests_in(11.0), None, "nothing waits any more");
        // On into the menu beside it, and on to the third.
        chain.pointed(1, Some(3), 11.0);
        assert!(chain.rested(11.3));
        assert_eq!(chain.levels.len(), 3);
        assert_eq!(open_lines(&chain), [Some(1), Some(3), None]);
    }

    #[test]
    fn crossing_a_menu_on_the_way_to_the_one_beside_it_closes_nothing() {
        let mut chain = Chain::of(menu(), false);
        chain.pressed(0, 1);
        assert_eq!(chain.levels.len(), 2);
        // Over another line of the first menu for less than a moment...
        chain.pointed(0, Some(3), 20.0);
        assert!(!chain.rested(20.1));
        // ...and into the menu that is open: it stays.
        chain.pointed(0, None, 20.12);
        chain.pointed(1, Some(0), 20.15);
        assert!(!chain.rested(21.0));
        assert_eq!(chain.levels.len(), 2);
        assert_eq!(open_lines(&chain), [Some(1), None]);
    }

    #[test]
    fn resting_on_another_line_closes_what_was_open_and_opens_what_that_line_leads_to() {
        let mut chain = Chain::of(menu(), false);
        chain.pressed(0, 1);
        chain.pressed(1, 3);
        assert_eq!(chain.levels.len(), 3);
        // Back on a line of the first menu that leads nowhere: one way
        // through the menus is open at a time, and this is none.
        chain.pointed(0, Some(0), 30.0);
        assert!(chain.rested(30.5));
        assert_eq!(chain.levels.len(), 1);
        assert_eq!(open_lines(&chain), [None]);
        // On a line that leads elsewhere: that menu, and only that.
        chain.pressed(0, 1);
        chain.pointed(0, Some(3), 31.0);
        assert!(chain.rested(31.5));
        assert_eq!(open_lines(&chain), [Some(3), None]);
        assert_eq!(chain.levels[1].lines, [choice("Solid")]);
        // Back on the line whose menu is open: it stays open.
        chain.pointed(0, Some(3), 32.0);
        assert!(!chain.rested(33.0));
        assert_eq!(chain.levels.len(), 2);
    }

    #[test]
    fn a_press_opens_at_once_and_a_line_that_does_something_is_what_was_chosen() {
        let mut chain = Chain::of(menu(), false);
        assert_eq!(chain.pressed(0, 1), None);
        assert_eq!(chain.levels.len(), 2);
        // Pressed again: still open, not opened twice.
        let before = chain.levels[1].serial;
        assert_eq!(chain.pressed(0, 1), None);
        assert_eq!(chain.levels[1].serial, before);
        assert_eq!(chain.pressed(1, 1), Some(does("List")));
        assert_eq!(chain.pressed(1, 2), None, "a gap is nothing to press");
        assert_eq!(chain.pressed(7, 0), None);
    }

    #[test]
    fn the_arrows_walk_the_lines_and_skip_the_gaps() {
        let mut chain = Chain::of(menu(), true);
        let at = |chain: &Chain| chain.levels.last().unwrap().at;
        assert_eq!(at(&chain), Some(0), "asked for by a key: on its first line");
        chain.key(Key::Down);
        chain.key(Key::Down);
        assert_eq!(at(&chain), Some(3), "over the gap");
        chain.key(Key::Down);
        chain.key(Key::Down);
        assert_eq!(at(&chain), Some(0), "round at the end");
        chain.key(Key::Up);
        assert_eq!(at(&chain), Some(4));
        chain.key(Key::First);
        assert_eq!(at(&chain), Some(0));
        chain.key(Key::Last);
        assert_eq!(at(&chain), Some(4));
        // Opened by the pointer, the keys begin at the first line going
        // down and at the last going up.
        let mut chain = Chain::of(menu(), false);
        assert_eq!(at(&chain), None);
        chain.key(Key::Up);
        assert_eq!(at(&chain), Some(4));
    }

    #[test]
    fn right_and_enter_go_into_a_menu_left_and_escape_come_back() {
        let mut chain = Chain::of(menu(), true);
        // Right on a line that leads nowhere is nothing.
        chain.key(Key::Into);
        assert_eq!(chain.levels.len(), 1);
        chain.key(Key::Down);
        chain.key(Key::Into);
        assert_eq!(chain.levels.len(), 2);
        assert_eq!(
            chain.levels[1].at,
            Some(0),
            "the keys are on its first line"
        );
        chain.key(Key::Last);
        assert_eq!(
            chain.key(Key::Choose),
            None,
            "Enter on a line with an arrow goes in"
        );
        assert_eq!(chain.levels.len(), 3);
        // Left: one back, and the keys are on the line that led there.
        chain.key(Key::Back);
        assert_eq!(chain.levels.len(), 2);
        assert_eq!((chain.levels[1].at, chain.levels[1].open), (Some(3), None));
        // Escape: one back each time, and from the first all are closed.
        chain.key(Key::Escape);
        assert_eq!(chain.levels.len(), 1);
        // Left in the first menu is nothing.
        chain.key(Key::Back);
        assert_eq!(chain.levels.len(), 1);
        chain.key(Key::Escape);
        assert!(chain.levels.is_empty());
        assert_eq!(chain.key(Key::Down), None, "no menu is open");
    }

    #[test]
    fn enter_chooses_the_line_the_keys_are_on() {
        let mut chain = Chain::of(menu(), true);
        assert_eq!(chain.key(Key::Choose), Some(does("Rename")));
        chain.key(Key::Down);
        chain.key(Key::Into);
        chain.key(Key::Down);
        assert_eq!(chain.key(Key::Choose), Some(does("List")));
        // On no line: nothing.
        let mut chain = Chain::of(menu(), false);
        assert_eq!(chain.key(Key::Choose), None);
    }

    #[test]
    fn each_menu_is_told_from_every_other_that_was_open_at_its_depth() {
        let mut chain = Chain::of(menu(), false);
        chain.pressed(0, 1);
        let first = chain.levels[1].serial;
        chain.pressed(0, 3);
        let second = chain.levels[1].serial;
        assert_ne!(first, second);
        assert_eq!(chain.level_of(first), None);
        assert_eq!(chain.level_of(second), Some(1));
    }

    #[test]
    fn every_line_has_its_place_and_the_menu_is_as_high_as_its_lines() {
        let lines = menu();
        let rows = rows(&lines);
        assert_eq!(rows.len(), lines.len());
        assert_eq!(
            rows[0],
            Rect::from_min_size(pos2(6.0, 6.0), vec2(224.0, 28.0))
        );
        assert_eq!(rows[1].top(), 36.0);
        // The gap is lower than a line.
        assert_eq!((rows[2].top(), rows[2].height()), (66.0, 7.0));
        assert_eq!(rows[3].top(), 75.0);
        let size = menu_size(&lines);
        assert_eq!(size, vec2(236.0, 12.0 + 4.0 * 30.0 + 9.0));
        assert!(rows.last().unwrap().bottom() <= size.y - 6.0);
    }
}
