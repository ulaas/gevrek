//! What the live lists share (events, the log, the windows): the search
//! over what a row says, the lowest level of the log that is shown, and
//! the rows as text for the clipboard.

use crate::link::Level;

/// Whether a row is found by what was typed: every word typed is in one of
/// the row's parts, whatever the case. Nothing typed finds every row.
pub fn found(typed: &str, parts: &[&str]) -> bool {
    let parts: Vec<String> = parts.iter().map(|part| part.to_lowercase()).collect();
    typed
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|word| parts.iter().any(|part| part.contains(&word)))
}

/// The levels from the least to the most serious.
pub const LEVELS: [Level; 4] = [Level::Debug, Level::Info, Level::Warning, Level::Error];

fn rank(level: Level) -> u8 {
    match level {
        Level::Debug => 0,
        Level::Info => 1,
        Level::Warning => 2,
        Level::Error => 3,
    }
}

/// Whether a line of this level is shown when `lowest` is the lowest level
/// asked for.
pub fn at_least(level: Level, lowest: Level) -> bool {
    rank(level) >= rank(lowest)
}

pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::Debug => "debug",
        Level::Info => "info",
        Level::Warning => "warning",
        Level::Error => "error",
    }
}

/// The time of day of a row (UTC), to the millisecond.
pub fn clock(time_ms: i64) -> String {
    let seconds = time_ms.div_euclid(1000);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds.div_euclid(3600).rem_euclid(24),
        seconds.div_euclid(60).rem_euclid(60),
        seconds.rem_euclid(60),
        time_ms.rem_euclid(1000)
    )
}

/// An event as one line of text, as the page shows it.
pub fn event_line(time_ms: i64, kind: &str, window: Option<u64>, fields: &str) -> String {
    let mut line = format!("{}  {kind}", clock(time_ms));
    if let Some(window) = window {
        line.push_str(&format!("  window {window}"));
    }
    if !fields.is_empty() {
        line.push_str("  ");
        line.push_str(fields);
    }
    line
}

/// A line of the log as text, with its level spelt out (on the page the
/// colour says it).
pub fn log_line(time_ms: i64, level: Level, message: &str) -> String {
    format!("{}  {:<7}  {message}", clock(time_ms), level_name(level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_typed_has_to_be_somewhere_in_the_row() {
        let row = ["WindowFocused", "title=Notes class=Notepad"];
        assert!(found("", &row));
        assert!(found("   ", &row));
        assert!(found("focused", &row));
        assert!(
            found("NOTEPAD window", &row),
            "words may be in different parts"
        );
        assert!(!found("focused firefox", &row));
        assert!(!found("x", &[]));
    }

    #[test]
    fn the_lowest_level_shown_hides_what_is_less_serious() {
        let shown = |lowest: Level| -> Vec<Level> {
            LEVELS
                .into_iter()
                .filter(|level| at_least(*level, lowest))
                .collect()
        };
        assert_eq!(shown(Level::Debug).len(), 4);
        assert_eq!(shown(Level::Warning), vec![Level::Warning, Level::Error]);
        assert_eq!(shown(Level::Error), vec![Level::Error]);
    }

    #[test]
    fn rows_are_written_for_the_clipboard_as_they_are_shown() {
        // 01:02:03.004 of some day.
        let time = ((24 + 1) * 3600 + 2 * 60 + 3) * 1000 + 4;
        assert_eq!(clock(time), "01:02:03.004");
        assert_eq!(
            event_line(time, "WindowFocused", Some(42), "title=Notes"),
            "01:02:03.004  WindowFocused  window 42  title=Notes"
        );
        assert_eq!(
            event_line(time, "RuntimeStarted", None, ""),
            "01:02:03.004  RuntimeStarted"
        );
        assert_eq!(
            log_line(time, Level::Warning, "slow"),
            "01:02:03.004  warning  slow"
        );
        assert_eq!(
            log_line(time, Level::Info, "ok"),
            "01:02:03.004  info     ok"
        );
    }
}
