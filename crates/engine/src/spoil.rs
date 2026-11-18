//! What is read from outside, spoilt: the numbers that look like chance,
//! the nonsense that is put where a value belongs, and the run that
//! tells whether the program came to its end.
//!
//! The file needs `serde_json` and nothing of the engine, so the tests of
//! another crate take it in as it is:
//!
//! `#[path = "../../engine/src/spoil.rs"] mod spoil;`

#![allow(dead_code)]

use serde_json::Value;

/// Numbers that look like chance and are the same in every run.
pub struct Chance(pub u64);

impl Chance {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, count: usize) -> usize {
        (self.next() % count.max(1) as u64) as usize
    }
}

/// How many times as often as usual, and with which seed.
pub fn harder() -> (usize, u64) {
    let times = std::env::var("GEVREK_SPOIL")
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    (times, (times as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// What is put where a value belongs.
pub fn nonsense(chance: &mut Chance) -> Value {
    match chance.below(22) {
        0 => Value::Null,
        1 => Value::Bool(true),
        2 => Value::Bool(false),
        3 => serde_json::json!(0),
        4 => serde_json::json!(-1),
        5 => serde_json::json!(1),
        6 => serde_json::json!(i64::MIN),
        7 => serde_json::json!(i64::MAX),
        8 => serde_json::json!(u64::MAX),
        9 => serde_json::json!(i32::MAX),
        10 => serde_json::json!(i32::MIN),
        11 => serde_json::json!(1e308),
        12 => serde_json::json!(-1e308),
        13 => serde_json::json!(0.000_000_1),
        14 => serde_json::json!(""),
        15 => serde_json::json!("\u{0}\u{feff}\n"),
        16 => serde_json::json!("x".repeat(5000)),
        17 => serde_json::json!("(unclosed[regex"),
        18 => serde_json::json!([]),
        19 => serde_json::json!({}),
        20 => serde_json::json!([[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]),
        _ => serde_json::json!(65536),
    }
}

/// Every place in a document, as the keys and indices that lead there.
pub fn places(value: &Value, here: &mut Vec<String>, all: &mut Vec<Vec<String>>) {
    all.push(here.clone());
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                here.push(key.clone());
                places(inner, here, all);
                here.pop();
            }
        }
        Value::Array(list) => {
            for (index, inner) in list.iter().enumerate() {
                here.push(index.to_string());
                places(inner, here, all);
                here.pop();
            }
        }
        _ => {}
    }
}

pub fn at<'a>(value: &'a mut Value, place: &[String]) -> Option<&'a mut Value> {
    let mut now = value;
    for step in place {
        now = match now {
            Value::Object(map) => map.get_mut(step)?,
            Value::Array(list) => list.get_mut(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(now)
}

/// One document spoilt: a value replaced by nonsense, a number made
/// another, a list emptied, doubled or made very long, a key taken out.
/// Returns what was done, to be told where it ends the program.
pub fn spoil(document: &mut Value, all: &[Vec<String>], chance: &mut Chance) -> String {
    let place = &all[chance.below(all.len())];
    let Some(value) = at(document, place) else {
        return "nothing".to_string();
    };
    let how = chance.below(8);
    let what = match (how, &mut *value) {
        (0..=3, value) => {
            *value = nonsense(chance);
            format!("= {}", short(value))
        }
        (4, Value::Number(number)) => {
            let by = [-1.0, 0.0, 1000.0, -1000.0, 0.5, 1e9][chance.below(6)];
            let now = number.as_f64().unwrap_or(0.0) * by;
            *value = serde_json::Number::from_f64(now.trunc())
                .map_or(Value::Null, Value::Number);
            format!("times {by}")
        }
        (5, Value::Array(list)) => {
            list.clear();
            "emptied".to_string()
        }
        (6, Value::Array(list)) if !list.is_empty() => {
            let twice = list.clone();
            for _ in 0..chance.below(40) {
                list.extend(twice.iter().cloned());
            }
            format!("made {} long", list.len())
        }
        (7, Value::Object(map)) if !map.is_empty() => {
            let key = map.keys().nth(chance.below(map.len())).cloned().unwrap_or_default();
            map.remove(&key);
            format!("without {key}")
        }
        (_, value) => {
            *value = nonsense(chance);
            format!("= {}", short(value))
        }
    };
    format!("/{} {what}", place.join("/"))
}

pub fn short(value: &Value) -> String {
    let text = value.to_string();
    text.chars().take(40).collect()
}

/// Runs `run` and tells whether it came to its end. (What a panic would
/// print is not printed: thousands of them are expected to be none.)
pub fn lived(run: impl FnOnce() + std::panic::UnwindSafe) -> Result<(), String> {
    std::panic::catch_unwind(run).map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "a panic".to_string())
    })
}
