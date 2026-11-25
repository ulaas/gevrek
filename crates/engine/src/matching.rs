//! Matching windows against a pattern: by title, class, executable or path,
//! and by the title or class a window had when it appeared.

use crate::contract::Desktop;
use crate::rules::{MatchKind, MatchStrategy};
use crate::WindowHandle;

/// The regular expression of a pattern, made once and kept: a rule is
/// held against every window there is, after each reload and for every
/// new window, and making the expression costs a thousand times what
/// matching with it does. `None`: the pattern does not compile (kept
/// too, so that it is not tried again and again).
pub fn regex_for(pattern: &str, case_sensitive: bool) -> Option<regex::Regex> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Kept = HashMap<(String, bool), Option<regex::Regex>>;
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    // (Patterns come from the configuration, and from commands too: no
    // more are kept than a configuration could hold.)
    const MOST: usize = 512;
    let mut kept = KEPT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(regex) = kept.get(&(pattern.to_string(), case_sensitive)) {
        return regex.clone();
    }
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .ok();
    if kept.len() >= MOST {
        kept.clear();
    }
    kept.insert((pattern.to_string(), case_sensitive), regex.clone());
    regex
}

/// Whether `value` matches `pattern` under `strategy`. A regular expression
/// that does not compile matches nothing.
pub fn text_matches(
    value: &str,
    pattern: &str,
    strategy: &MatchStrategy,
    case_sensitive: bool,
) -> bool {
    if matches!(strategy, MatchStrategy::Regex) {
        return regex_for(pattern, case_sensitive).is_some_and(|regex| regex.is_match(value));
    }
    let (value, pattern) = if case_sensitive {
        (value.to_string(), pattern.to_string())
    } else {
        (value.to_lowercase(), pattern.to_lowercase())
    };
    match strategy {
        MatchStrategy::Equals => value == pattern,
        MatchStrategy::Contains => value.contains(&pattern),
        MatchStrategy::StartsWith => value.starts_with(&pattern),
        MatchStrategy::EndsWith => value.ends_with(&pattern),
        MatchStrategy::Regex => unreachable!("handled above"),
    }
}

/// Whether a window's title, class, executable or path matches. Other
/// kinds of match (window state) never match here.
pub fn window_matches(
    desktop: &dyn Desktop,
    id: WindowHandle,
    pattern: &str,
    kind: &MatchKind,
    strategy: &MatchStrategy,
    case_sensitive: bool,
) -> bool {
    let value = match kind {
        MatchKind::Title => desktop.title(id),
        MatchKind::Class => desktop.class(id),
        MatchKind::Exe => desktop.exe(id),
        MatchKind::Path => desktop.path(id),
        MatchKind::AppId => desktop.app_id(id),
        _ => return false,
    };
    text_matches(&value, pattern, strategy, case_sensitive)
}

/// Every tracked window that matches.
pub fn find_windows_by_match(
    desktop: &dyn Desktop,
    pattern: &str,
    kind: &MatchKind,
    strategy: &MatchStrategy,
    case_sensitive: bool,
) -> Vec<WindowHandle> {
    desktop
        .windows()
        .into_iter()
        .map(|window| window.handle)
        .filter(|id| window_matches(desktop, *id, pattern, kind, strategy, case_sensitive))
        .collect()
}

/// Every tracked window whose class (`class`) or title, as it was when the
/// window appeared, matches.
pub fn find_windows_by_initial_match(
    desktop: &dyn Desktop,
    pattern: &str,
    class: bool,
    strategy: &MatchStrategy,
    case_sensitive: bool,
) -> Vec<WindowHandle> {
    desktop
        .windows()
        .into_iter()
        .map(|window| window.handle)
        .filter(|id| {
            let value = if class {
                desktop.initial_class(*id)
            } else {
                desktop.initial_title(*id)
            };
            text_matches(&value, pattern, strategy, case_sensitive)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_matching_follows_the_strategy_and_the_case_setting() {
        use MatchStrategy::*;
        assert!(text_matches("Notepad", "notepad", &Equals, false));
        assert!(!text_matches("Notepad", "notepad", &Equals, true));
        assert!(text_matches(
            "Untitled - Notepad",
            "notepad",
            &Contains,
            false
        ));
        assert!(text_matches(
            "Untitled - Notepad",
            "untitled",
            &StartsWith,
            false
        ));
        assert!(text_matches(
            "Untitled - Notepad",
            "Notepad",
            &EndsWith,
            true
        ));
        assert!(!text_matches(
            "Untitled - Notepad",
            "Untitled",
            &EndsWith,
            true
        ));
        assert!(text_matches(
            "report-2026.txt",
            r"^report-\d{4}",
            &Regex,
            true
        ));
        assert!(text_matches(
            "REPORT-2026.txt",
            r"^report-\d{4}",
            &Regex,
            false
        ));
        assert!(
            !text_matches("anything", "(unclosed", &Regex, false)
                && !text_matches("anything", "(unclosed", &Regex, false),
            "a bad pattern matches nothing"
        );
    }

    #[test]
    fn an_expression_that_is_kept_matches_as_one_made_anew() {
        use MatchStrategy::Regex;
        // The same pattern with and without regard to case: two
        // expressions, kept apart, each the same every time it is asked.
        for _ in 0..3 {
            assert!(text_matches(
                "REPORT-2026.txt",
                r"^report-\d{4}",
                &Regex,
                false
            ));
            assert!(!text_matches(
                "REPORT-2026.txt",
                r"^report-\d{4}",
                &Regex,
                true
            ));
            assert!(text_matches(
                "report-2026.txt",
                r"^report-\d{4}",
                &Regex,
                true
            ));
        }
        // More patterns than are kept: the early ones still match.
        for index in 0..1200 {
            let pattern = format!("^window {index}$");
            assert!(text_matches(
                &format!("window {index}"),
                &pattern,
                &Regex,
                true
            ));
            assert!(!text_matches(
                &format!("window {}", index + 1),
                &pattern,
                &Regex,
                true
            ));
        }
        assert!(text_matches("window 0", "^window 0$", &Regex, true));
        assert!(regex_for("(unclosed", true).is_none());
    }
}
