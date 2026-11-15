//! The configuration as text (the Raw page): what each piece of the text
//! is, for its colour, and on which line each place of the configuration
//! stands, so that a problem can point at its line.
//!
//! The text is the person's while they type: it may be half written and
//! not JSON at all. Nothing here refuses such text; what cannot be read is
//! passed over.

use std::collections::HashMap;

/// What a piece of the text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The name of a setting (a string before a colon).
    Key,
    /// A string that is a value.
    Text,
    Number,
    /// `true`, `false`, `null`.
    Word,
    /// Brackets, commas, colons.
    Mark,
    /// Anything else (also what is not JSON).
    Other,
}

/// A piece of the text: bytes `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

/// The text in pieces, in order and without gaps except for white space.
pub fn pieces(text: &str) -> Vec<Piece> {
    let bytes = text.as_bytes();
    let mut found: Vec<Piece> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let kind = match bytes[at] {
            byte if byte.is_ascii_whitespace() => {
                at += 1;
                continue;
            }
            b'"' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    // An escaped character, whatever it is, is part of the string.
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at = (at + 1).min(bytes.len());
                Kind::Text
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                at += 1;
                Kind::Mark
            }
            byte if byte == b'-' || byte.is_ascii_digit() => {
                at += 1;
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric()
                        || matches!(bytes[at], b'.' | b'+' | b'-'))
                {
                    at += 1;
                }
                Kind::Number
            }
            byte if byte.is_ascii_alphabetic() => {
                while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                    at += 1;
                }
                match &text[start..at] {
                    "true" | "false" | "null" => Kind::Word,
                    _ => Kind::Other,
                }
            }
            _ => {
                // One whole character, of however many bytes.
                at += 1;
                while at < bytes.len() && !text.is_char_boundary(at) {
                    at += 1;
                }
                Kind::Other
            }
        };
        // A string followed by a colon names a setting.
        if bytes[start] == b':' {
            if let Some(before) = found.last_mut().filter(|piece| piece.kind == Kind::Text) {
                before.kind = Kind::Key;
            }
        }
        found.push(Piece {
            kind,
            start,
            end: at,
        });
    }
    found
}

/// The line (from 0) each place of the configuration begins on, by its
/// JSON pointer (`/desktop/focus/history_size`, `/desktop/window_rules/2`).
/// The whole text is `""`.
pub fn lines(text: &str) -> HashMap<String, usize> {
    /// What the walk is inside of.
    enum Inside {
        /// An object, with the key the next value belongs to.
        Object(Option<String>),
        /// A list, with the index of the next value.
        List(usize),
    }
    fn pointer(stack: &[Inside]) -> String {
        let mut pointer = String::new();
        for level in stack {
            pointer.push('/');
            match level {
                Inside::Object(key) => pointer.push_str(
                    &key.as_deref()
                        .unwrap_or("")
                        .replace('~', "~0")
                        .replace('/', "~1"),
                ),
                Inside::List(index) => pointer.push_str(&index.to_string()),
            }
        }
        pointer
    }

    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let line_of = |byte: usize| line_starts.partition_point(|start| *start <= byte) - 1;

    let mut found = HashMap::new();
    let mut stack: Vec<Inside> = Vec::new();
    for piece in pieces(text) {
        let word = &text[piece.start..piece.end];
        // Where a value begins, its place is known.
        let begins_a_value = match piece.kind {
            Kind::Key => {
                if let Some(Inside::Object(key)) = stack.last_mut() {
                    *key = serde_json::from_str::<String>(word).ok();
                }
                false
            }
            Kind::Mark => matches!(word, "{" | "["),
            Kind::Text | Kind::Number | Kind::Word => true,
            Kind::Other => false,
        };
        if begins_a_value {
            found
                .entry(pointer(&stack))
                .or_insert_with(|| line_of(piece.start));
        }
        match word {
            "{" => stack.push(Inside::Object(None)),
            "[" => stack.push(Inside::List(0)),
            "}" | "]" => {
                stack.pop();
            }
            "," => {
                if let Some(Inside::List(index)) = stack.last_mut() {
                    *index += 1;
                }
            }
            _ => {}
        }
    }
    found
}

/// Where the cursor stands in the text, as far as completing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// The place of the object the cursor is in (`/desktop/focus`), or,
    /// for a value, the place of the value itself.
    pub pointer: String,
    /// A key of the object is being written (else: a value that is a
    /// text).
    pub key: bool,
    /// What is written of it so far, without its quote.
    pub typed: String,
    /// Where what is written begins (its opening quote, where it has
    /// one), in bytes of the text.
    pub start: usize,
    /// The keys the object has already.
    pub present: Vec<String>,
}

/// Where a cursor after `at` bytes of the text stands: in a key that is
/// being written (or where one may begin), or in a text that is a value.
/// `None` anywhere else (in a number, between values, outside of all).
pub fn standing(text: &str, at: usize) -> Option<Standing> {
    enum Inside {
        /// An object: the keys it has, the key the next value belongs
        /// to, and whether a key is what comes next.
        Object(Vec<String>, Option<String>, bool),
        List(usize),
    }
    fn pointer(stack: &[Inside], with_key: bool) -> String {
        let mut pointer = String::new();
        for (depth, level) in stack.iter().enumerate() {
            match level {
                Inside::Object(_, key, _) => {
                    // The innermost object itself is the place when a key
                    // is being written.
                    if depth + 1 == stack.len() && !with_key {
                        break;
                    }
                    pointer.push('/');
                    pointer.push_str(
                        &key.as_deref()
                            .unwrap_or("")
                            .replace('~', "~0")
                            .replace('/', "~1"),
                    );
                }
                Inside::List(index) => {
                    pointer.push('/');
                    pointer.push_str(&index.to_string());
                }
            }
        }
        pointer
    }
    let at = at.min(text.len());
    let bytes = text.as_bytes();
    let mut stack: Vec<Inside> = Vec::new();
    let mut index = 0;
    while index < at {
        match bytes[index] {
            b'"' => {
                // A text: to its end, or to the cursor when that is in it.
                let start = index;
                let mut end = index + 1;
                let mut closed = false;
                while end < at {
                    match bytes[end] {
                        b'\\' => end += 1,
                        b'"' => {
                            closed = true;
                            break;
                        }
                        _ => {}
                    }
                    end += 1;
                }
                let is_key = matches!(stack.last(), Some(Inside::Object(_, _, true)));
                if !closed {
                    let typed = text.get(start + 1..at)?.to_string();
                    return Some(match stack.last() {
                        Some(Inside::Object(present, _, true)) => Standing {
                            pointer: pointer(&stack, false),
                            key: true,
                            typed,
                            start,
                            // Those before the cursor, and those after.
                            present: present
                                .iter()
                                .cloned()
                                .chain(keys_after(text, at, true))
                                .collect(),
                        },
                        _ => Standing {
                            pointer: pointer(&stack, true),
                            key: false,
                            typed,
                            start,
                            present: Vec::new(),
                        },
                    });
                }
                if is_key {
                    let word = serde_json::from_str::<String>(text.get(start..=end)?).ok();
                    if let Some(Inside::Object(present, key, wants_key)) = stack.last_mut() {
                        if let Some(word) = &word {
                            present.push(word.clone());
                        }
                        *key = word;
                        *wants_key = false;
                    }
                }
                index = end + 1;
                continue;
            }
            b'{' => stack.push(Inside::Object(Vec::new(), None, true)),
            b'[' => stack.push(Inside::List(0)),
            b'}' | b']' => {
                stack.pop();
            }
            b',' => match stack.last_mut() {
                Some(Inside::Object(_, _, wants_key)) => *wants_key = true,
                Some(Inside::List(index)) => *index += 1,
                None => {}
            },
            _ => {}
        }
        index += 1;
    }
    // Not in a text: a key may begin here when the object waits for one
    // and nothing but room stands between its last mark and the cursor.
    match stack.last() {
        Some(Inside::Object(present, _, true)) if text[..at].trim_end().ends_with(['{', ',']) => {
            Some(Standing {
                pointer: pointer(&stack, false),
                key: true,
                typed: String::new(),
                start: at,
                present: present
                    .iter()
                    .cloned()
                    .chain(keys_after(text, at, false))
                    .collect(),
            })
        }
        _ => None,
    }
}

/// The keys an object has after a place in it (`from`, in bytes), up to
/// where the object ends. `in_text`: the place is inside a text, whose
/// rest (to its closing quote on the same line, where it has one) is not
/// looked at.
fn keys_after(text: &str, from: usize, in_text: bool) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut index = from.min(bytes.len());
    if in_text {
        // The rest of the text the cursor is in.
        let line_end = text[index..]
            .find('\n')
            .map_or(bytes.len(), |at| index + at);
        match text[index..line_end].find('"') {
            Some(at) => index += at + 1,
            None => index = line_end,
        }
    }
    let mut found = Vec::new();
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index;
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != b'"' {
                    if bytes[end] == b'\\' {
                        end += 1;
                    }
                    end += 1;
                }
                if end >= bytes.len() {
                    break;
                }
                // A key of this object: a text with a colon after it.
                if depth == 0 && text[end + 1..].trim_start().starts_with(':') {
                    if let Ok(key) = serde_json::from_str::<String>(&text[start..=end]) {
                        found.push(key);
                    }
                }
                index = end;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        index += 1;
    }
    found
}

/// What can be written where the cursor stands, by the configuration's
/// own description of itself: the keys an object has and does not have
/// yet, or the words a value is one of. Each with what it means; those
/// that begin with what is written first, then those that contain it.
pub fn offers(standing: &Standing) -> Vec<(String, String)> {
    let Some(node) = gevrek_schema::schema().at(&standing.pointer) else {
        return Vec::new();
    };
    fn unwrapped(node: gevrek_schema::Node<'_>) -> gevrek_schema::Kind<'_> {
        match node.kind() {
            gevrek_schema::Kind::Optional(inner) => unwrapped(inner),
            kind => kind,
        }
    }
    let first_line = |text: &str| text.lines().next().unwrap_or("").to_string();
    let mut all: Vec<(String, String)> = match (unwrapped(node), standing.key) {
        (gevrek_schema::Kind::Object(fields), true) => fields
            .iter()
            .filter(|field| !standing.present.iter().any(|known| known == field.name))
            .map(|field| (field.name.to_string(), first_line(field.node.description())))
            .collect(),
        (gevrek_schema::Kind::Choice(choices), false) => choices
            .iter()
            .map(|choice| (choice.value.to_string(), first_line(choice.description)))
            .collect(),
        _ => Vec::new(),
    };
    let typed = standing.typed.to_lowercase();
    all.retain(|(word, _)| word.to_lowercase().contains(&typed));
    all.sort_by_key(|(word, _)| (!word.to_lowercase().starts_with(&typed), word.clone()));
    all
}

/// The text with an offer written where the cursor stands, and where the
/// cursor is afterwards (in bytes): a key with its colon, a word with its
/// closing quote. What was written of it so far, up to `at`, is replaced.
pub fn completed(text: &str, at: usize, standing: &Standing, word: &str) -> (String, usize) {
    let at = at.min(text.len());
    let start = standing.start.min(at);
    let mut written = serde_json::to_string(word).unwrap_or_else(|_| format!("\"{word}\""));
    // A closing quote that is there already (the editor writes them in
    // pairs) is the word's own.
    let after = if text[at..].starts_with('"') {
        at + 1
    } else {
        at
    };
    if standing.key && !text[after..].trim_start().starts_with(':') {
        written.push_str(": ");
    }
    let mut out = String::with_capacity(text.len() + written.len());
    out.push_str(&text[..start]);
    out.push_str(&written);
    let cursor = out.len();
    out.push_str(&text[after..]);
    (out, cursor)
}

/// The line a problem at `pointer` is shown on: the place itself, or when
/// it is not written in the text (a setting that is missing) the nearest
/// place above it that is.
pub fn line_of(lines: &HashMap<String, usize>, pointer: &str) -> Option<usize> {
    let mut pointer = pointer;
    loop {
        if let Some(line) = lines.get(pointer) {
            return Some(*line);
        }
        pointer = &pointer[..pointer.rfind('/')?];
    }
}

/// The number of characters before the beginning of a line (from 0): where
/// the text cursor goes to stand on it.
pub fn chars_before_line(text: &str, line: usize) -> usize {
    text.split_inclusive('\n')
        .take(line)
        .map(|line| line.chars().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"{
  "version": "3",
  "desktop": {
    "focus": {
      "history_size": 20,
      "follow": true
    },
    "window_rules": [
      {
        "name": "a/b ü"
      },
      {
        "name": "second",
        "effects": []
      }
    ]
  }
}"#;

    #[test]
    fn every_piece_of_the_text_is_what_it_is() {
        let kinds: Vec<(Kind, &str)> = pieces(r#"{"a": [1.5e3, true, "x:\"y\""], "b": nul}"#)
            .into_iter()
            .map(|piece| {
                (
                    piece.kind,
                    &r#"{"a": [1.5e3, true, "x:\"y\""], "b": nul}"#[piece.start..piece.end],
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Mark, "{"),
                (Kind::Key, "\"a\""),
                (Kind::Mark, ":"),
                (Kind::Mark, "["),
                (Kind::Number, "1.5e3"),
                (Kind::Mark, ","),
                (Kind::Word, "true"),
                (Kind::Mark, ","),
                (Kind::Text, r#""x:\"y\"""#),
                (Kind::Mark, "]"),
                (Kind::Mark, ","),
                (Kind::Key, "\"b\""),
                (Kind::Mark, ":"),
                (Kind::Other, "nul"),
                (Kind::Mark, "}"),
            ]
        );
    }

    #[test]
    fn text_that_is_not_json_is_passed_over_without_harm() {
        for text in [
            "",
            "{",
            "\"never closed",
            "{\"a\": ü§ }",
            "]]]]",
            "\"ends in a backslash\\",
        ] {
            let found = pieces(text);
            assert!(found
                .iter()
                .all(|piece| piece.start < piece.end && piece.end <= text.len()));
            assert!(found.iter().all(
                |piece| text.is_char_boundary(piece.start) && text.is_char_boundary(piece.end)
            ));
            lines(text);
        }
    }

    #[test]
    fn the_cursor_knows_whether_a_key_or_a_word_is_being_written() {
        let text = r#"{"desktop": {"focus": {"history_size": 5, "fo"#;
        let found = standing(text, text.len()).expect("in a key");
        assert_eq!(found.pointer, "/desktop/focus");
        assert!(found.key);
        assert_eq!(found.typed, "fo");
        assert_eq!(&text[found.start..], "\"fo");
        assert_eq!(found.present, ["history_size"]);
        // Where a key may begin, nothing written yet.
        let text = "{\"desktop\": {\"focus\": {\n    ";
        let found = standing(text, text.len()).expect("a key may begin");
        assert_eq!(
            (found.pointer.as_str(), found.typed.as_str()),
            ("/desktop/focus", "")
        );
        assert_eq!(found.start, text.len());
        // A value that is a text, in a list of objects.
        let text = r#"{"desktop": {"workspaces": [{"name": "1"}, {"layout": "dw"#;
        let found = standing(text, text.len()).expect("in a value");
        assert_eq!(found.pointer, "/desktop/workspaces/1/layout");
        assert!(!found.key);
        assert_eq!(found.typed, "dw");
        // A quote inside a text does not end it; a cursor in the middle
        // of the text only knows what is before it.
        let text = r#"{"a": "x \" y", "b"#;
        assert_eq!(standing(text, text.len()).expect("a key").typed, "b");
        let whole = r#"{"desktop": {"focus": {}}}"#;
        let inside = whole.find("{}").expect("there") + 1;
        assert_eq!(
            standing(whole, inside).expect("a key may begin").pointer,
            "/desktop/focus"
        );
        // Not where a key or a text is: after a colon, in a number, outside.
        assert_eq!(standing(r#"{"a": "#, 6), None);
        assert_eq!(standing(r#"{"a": 12"#, 8), None);
        assert_eq!(standing("", 0), None);
        assert_eq!(standing(r#"{"a": 1}"#, 8), None);
    }

    #[test]
    fn the_keys_an_object_has_after_the_cursor_count_as_there_too() {
        let text = "{\"a\": 1,\n  \n  \"b\": {\"inner\": 2},\n  \"c\": [\"x: y\", {\"deep\": 1}]\n}, \"outside\": 3";
        let at = text.find("\n  \n").expect("there") + 3;
        let found = standing(text, at).expect("a key may begin");
        assert_eq!(found.present, ["a", "b", "c"]);
        // Written into a key that is closed already.
        let text = r#"{"a": 1, "fo": 2, "z": 3}"#;
        let at = text.find("fo").expect("there") + 1;
        let found = standing(text, at).expect("in a key");
        assert_eq!(found.typed, "f");
        assert_eq!(found.present, ["a", "z"]);
    }

    #[test]
    fn what_is_offered_is_what_the_configuration_has_there() {
        let text = r#"{"desktop": {"focus": {"history_size": 5, "fo"#;
        let found = standing(text, text.len()).expect("in a key");
        let keys = offers(&found);
        assert!(
            !keys.is_empty(),
            "the focus settings have keys with fo in them"
        );
        assert!(keys
            .iter()
            .all(|(key, _)| key.to_lowercase().contains("fo")));
        assert!(
            keys.iter().all(|(key, _)| key != "history_size"),
            "not one that is there"
        );
        // Those that begin with what is written come first.
        let begins: Vec<bool> = keys.iter().map(|(key, _)| key.starts_with("fo")).collect();
        assert!(begins.windows(2).all(|pair| pair[0] >= pair[1]), "{keys:?}");
        // The words a value is one of.
        let text = r#"{"desktop": {"workspaces": [{"layout": "dw"#;
        let words = offers(&standing(text, text.len()).expect("in a value"));
        // (Found whatever the case of what is written; written as the
        // configuration writes it.)
        assert_eq!(
            words.first().map(|(word, _)| word.as_str()),
            Some("Dwindle")
        );
        // A place the configuration does not have offers nothing.
        let text = r#"{"nowhere": {"x"#;
        assert!(offers(&standing(text, text.len()).expect("in a key")).is_empty());
    }

    #[test]
    fn an_offer_is_written_in_place_of_what_was_typed() {
        let text = r#"{"desktop": {"focus": {"fo"#;
        let found = standing(text, text.len()).expect("in a key");
        let (written, cursor) = completed(text, text.len(), &found, "follow_mouse");
        assert_eq!(written, r#"{"desktop": {"focus": {"follow_mouse": "#);
        assert_eq!(cursor, written.len());
        // With the closing quote and the rest of the text after it.
        let text = r#"{"desktop": {"focus": {"fo"}}}"#;
        let at = text.rfind("fo").expect("there") + 2;
        let found = standing(text, at).expect("in a key");
        let (written, cursor) = completed(text, at, &found, "follow_mouse");
        assert_eq!(written, r#"{"desktop": {"focus": {"follow_mouse": }}}"#);
        assert_eq!(&written[cursor..], "}}}");
        // A word of a value: closed, no colon.
        let text = r#"{"layout": "dw"#;
        let found = standing(text, text.len()).expect("in a value");
        assert_eq!(
            completed(text, text.len(), &found, "Dwindle").0,
            r#"{"layout": "Dwindle""#
        );
        // A key where none is written yet.
        let text = "{";
        let found = standing(text, 1).expect("a key may begin");
        assert_eq!(completed(text, 1, &found, "desktop").0, r#"{"desktop": "#);
    }

    #[test]
    fn every_place_knows_its_line() {
        let found = lines(TEXT);
        assert_eq!(found[""], 0);
        assert_eq!(found["/version"], 1);
        assert_eq!(found["/desktop"], 2);
        assert_eq!(found["/desktop/focus/history_size"], 4);
        assert_eq!(found["/desktop/focus/follow"], 5);
        assert_eq!(found["/desktop/window_rules"], 7);
        assert_eq!(found["/desktop/window_rules/0"], 8);
        assert_eq!(found["/desktop/window_rules/0/name"], 9);
        assert_eq!(found["/desktop/window_rules/1/name"], 12);
        assert_eq!(found["/desktop/window_rules/1/effects"], 13);
        assert!(!found.contains_key("/desktop/window_rules/2"));
    }

    #[test]
    fn a_problem_at_a_place_not_written_points_at_the_nearest_one_above() {
        let found = lines(TEXT);
        assert_eq!(line_of(&found, "/desktop/focus/history_size"), Some(4));
        // Not in the text: the rule it would be in, then the list.
        assert_eq!(
            line_of(&found, "/desktop/window_rules/1/predicates/0/value"),
            Some(11)
        );
        assert_eq!(line_of(&found, "/desktop/window_rules/7"), Some(7));
        assert_eq!(line_of(&found, "/nothing/like/it"), Some(0));
        assert_eq!(line_of(&HashMap::new(), "/a"), None);
    }

    #[test]
    fn the_cursor_finds_the_beginning_of_a_line() {
        assert_eq!(chars_before_line(TEXT, 0), 0);
        assert_eq!(chars_before_line(TEXT, 1), 2);
        // Counted in characters, not bytes: line 9 has an ü.
        let before_ten = chars_before_line(TEXT, 10);
        let line_ten: String = TEXT.chars().skip(before_ten).take(7).collect();
        assert_eq!(line_ten, "      }");
    }
}
