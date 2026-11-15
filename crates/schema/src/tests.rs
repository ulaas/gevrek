use super::*;
use gevrek_model::Config;

fn every_own_configuration() -> Vec<(String, Value)> {
    let mut found = vec![(
        "the default".to_string(),
        serde_json::to_value(Config::default()).expect("a configuration is JSON"),
    )];
    found.push((
        "the starting configuration".to_string(),
        serde_json::to_value(gevrek_model::examples::starting_configuration()).expect("JSON"),
    ));
    for example in gevrek_model::examples::built_in_examples() {
        found.push((
            format!("the example {}", example.id),
            serde_json::to_value(&example.config).expect("JSON"),
        ));
    }
    found
}

/// The proof that forms made from the schema can show everything: there
/// is no value in any configuration gevrek itself writes that the schema
/// read through `Kind` does not explain.
#[test]
fn every_value_of_gevreks_own_configurations_is_explained() {
    let configurations = every_own_configuration();
    assert!(configurations.len() > 3, "the examples are read");
    for (name, value) in configurations {
        let problems = unexplained(schema().root(), &value);
        assert!(
            problems.is_empty(),
            "{name}: {} unexplained, first: {:?}",
            problems.len(),
            &problems[..problems.len().min(5)]
        );
    }
}

#[test]
fn nothing_the_model_knows_is_read_as_anything_goes() {
    // `Any` is for the keys of a newer gevrek and for free JSON a node
    // carries; a setting of the desktop must never end up there.
    let vague: Vec<String> = catalogue()
        .into_iter()
        .filter(|setting| setting.pointer.starts_with("/desktop") && setting.kind == "any")
        .map(|setting| setting.pointer)
        .collect();
    assert!(vague.is_empty(), "read as any JSON: {vague:?}");
}

#[test]
fn every_setting_has_a_place_in_the_default_configuration() {
    let default = serde_json::to_value(Config::default()).expect("JSON");
    let settings = catalogue();
    assert!(settings.len() > 100, "{} settings", settings.len());
    for setting in &settings {
        assert!(
            default.pointer(&setting.pointer).is_some(),
            "{} is in the schema and not in the default configuration",
            setting.pointer
        );
        assert!(
            schema().at(&setting.pointer).is_some(),
            "{} is found again by its pointer",
            setting.pointer
        );
    }
}

#[test]
fn every_number_says_how_to_offer_it() {
    let silent: Vec<String> = catalogue()
        .into_iter()
        // (The graph's counters are bookkeeping, not settings.)
        .filter(|setting| !setting.pointer.starts_with("/node_graph"))
        .filter(|setting| setting.kind == "number" && hint(&setting.pointer).is_none())
        .map(|setting| setting.pointer)
        .collect();
    assert!(
        silent.is_empty(),
        "numbers without a range (crates/schema/src/hints.rs): {silent:?}"
    );
}

#[test]
fn the_documentation_of_a_field_is_its_description() {
    let described = catalogue()
        .into_iter()
        .filter(|setting| !setting.description.is_empty())
        .count();
    let all = catalogue().len();
    // Not every field is documented in the model yet; most are, and the
    // number may only go up.
    assert!(
        described * 100 >= all * 55,
        "{described} of {all} settings have a description"
    );
    let enabled = schema()
        .at("/desktop/outputs/0/enabled")
        .expect("an output's switch");
    assert!(matches!(enabled.kind(), Kind::Bool));
    assert_eq!(enabled.description(), "Off: the display is not used.");
}

#[test]
fn kinds_are_what_a_form_expects() {
    let at = |pointer: &str| {
        schema()
            .at(pointer)
            .unwrap_or_else(|| panic!("{pointer} is in the schema"))
            .kind()
    };
    assert!(matches!(
        at("/desktop/appearance/active_border_color"),
        Kind::Color
    ));
    assert!(matches!(at("/desktop/outputs"), Kind::List(_)));
    assert!(matches!(at("/desktop/outputs/0/mode"), Kind::Optional(_)));
    assert!(matches!(
        at("/desktop/outputs/0/mode/width"),
        Kind::Integer { min: Some(0), .. }
    ));
    assert!(
        matches!(at("/desktop/outputs/0/scale"), Kind::Optional(inner) if matches!(inner.kind(), Kind::Number))
    );
    match at("/desktop/outputs/0/transform") {
        Kind::Choice(words) => {
            let names: Vec<&str> = words.iter().map(|word| word.value).collect();
            assert_eq!(names.len(), 4);
            assert!(names.contains(&"right"));
            let right = words
                .iter()
                .find(|word| word.value == "right")
                .expect("right");
            assert!(right.description.contains("quarter"));
        }
        _ => panic!("a display's turning is a choice"),
    }
    // A command names itself in `command` and has its own fields.
    match at("/desktop/bindings/0/command") {
        Kind::Union(variants) => {
            let exec = variants
                .iter()
                .find(|variant| variant.name == "exec")
                .expect("exec");
            match &exec.shape {
                VariantShape::Tagged { key, fields } => {
                    assert_eq!(*key, "command");
                    assert!(fields.iter().any(|field| field.name == "program"));
                }
                _ => panic!("a command is an object with its name in a key"),
            }
            let quit = variants
                .iter()
                .find(|variant| variant.name == "quit")
                .expect("quit");
            assert_eq!(quit.blank(), serde_json::json!({"command": "quit"}));
            assert!(quit.matches(&serde_json::json!({"command": "quit"})));
        }
        _ => panic!("a command is one of several forms"),
    }
}

#[test]
fn a_blank_value_is_one_the_model_reads() {
    // A new item of every list of the desktop, as the form would add it.
    let default = serde_json::to_value(Config::default()).expect("JSON");
    let mut lists = 0;
    for setting in catalogue()
        .into_iter()
        .filter(|setting| setting.kind == "list")
    {
        let Some(Kind::List(item)) = schema().at(&setting.pointer).map(|node| match node.kind() {
            Kind::Optional(inner) => inner.kind(),
            kind => kind,
        }) else {
            continue;
        };
        if !setting.pointer.starts_with("/desktop") {
            continue;
        }
        let mut changed = default.clone();
        let Some(Value::Array(items)) = changed.pointer_mut(&setting.pointer) else {
            continue;
        };
        items.push(item.blank());
        lists += 1;
        let read: Result<Config, _> = serde_json::from_value(changed);
        assert!(
            read.is_ok(),
            "a new item of {} is not read: {:?}",
            setting.pointer,
            read.err()
        );
    }
    assert!(lists >= 8, "{lists} lists were tried");
}

#[test]
fn names_read_as_titles() {
    assert_eq!(title("corner_radius"), "Corner radius");
    assert_eq!(title("duration_ms"), "Duration (ms)");
    assert_eq!(title("xwayland"), "Xwayland");
    assert_eq!(title("fps"), "FPS");
}

#[test]
fn what_stands_on_the_panel_is_a_list_of_words_to_choose_from() {
    let item = schema()
        .at("/desktop/shell/panel/left")
        .and_then(|list| list.child("0"))
        .expect("the panel's left side is a list");
    let Kind::Choice(words) = item.kind() else {
        panic!("a module is chosen from words");
    };
    let names: Vec<&str> = words.iter().map(|word| word.value).collect();
    assert!(
        names.contains(&"workspaces") && names.contains(&"clock"),
        "{names:?}"
    );
    assert!(
        words.iter().all(|word| !word.description.is_empty()),
        "each says what it is"
    );
}
