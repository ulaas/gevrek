//! One configuration format on every system: each built-in example, written
//! by this build, is byte for byte the file kept in `tests/golden`. CI runs
//! this on Linux and on Windows, so the two cannot drift apart.
//!
//! After a deliberate change of the format or of an example:
//!
//!     GEVREK_BLESS=1 cargo test -p gevrek-model --test conformance
//!
//! and review the difference in the golden files like any other change.

use gevrek_model::Config;
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// The terminals `default_terminal` names, one per system.
const TERMINALS: [&str; 2] = ["wt.exe", "kitty"];

/// The text a save writes, with the one value that differs by system (the
/// terminal Super+T starts, until named default programs exist) made the
/// same, and Unix line ends.
///
/// Every system's terminal is replaced, not only this one's: an example
/// that names Windows Terminal outright would otherwise be written
/// differently on Windows (where it is also the default) than elsewhere.
fn written(config: &Config) -> String {
    let mut text = serde_json::to_string_pretty(config).unwrap();
    for terminal in TERMINALS {
        text = text.replace(
            &format!("\"program\": \"{terminal}\""),
            "\"program\": \"@terminal\"",
        );
    }
    text.replace("\r\n", "\n") + "\n"
}

#[test]
fn the_default_terminal_is_one_the_golden_files_hide() {
    assert!(TERMINALS.contains(&gevrek_model::examples::default_terminal()));
}

#[test]
fn every_example_is_written_as_its_golden_file() {
    let bless = std::env::var_os("GEVREK_BLESS").is_some();
    let mut different = Vec::new();
    for example in gevrek_model::examples::built_in_examples() {
        let path = golden_dir().join(format!("{}.json", example.id));
        let text = written(&example.config);
        if bless {
            std::fs::create_dir_all(golden_dir()).unwrap();
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let golden = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if golden != text {
            different.push(example.id);
        }
    }
    assert!(
        different.is_empty(),
        "these examples are no longer written as their golden files: {different:?} \
         (GEVREK_BLESS=1 to accept, then review the diff)"
    );
}

#[test]
fn every_golden_file_loads_validates_and_is_written_back_unchanged() {
    let mut seen = 0;
    for entry in std::fs::read_dir(golden_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        let config = Config::from_json(&text.replace(
            "\"program\": \"@terminal\"",
            &format!(
                "\"program\": \"{}\"",
                gevrek_model::examples::default_terminal()
            ),
        ))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let errors: Vec<_> = config
            .validate()
            .into_iter()
            .filter(|issue| issue.severity == gevrek_model::settings::ConfigIssueSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        assert_eq!(written(&config), text, "{}", path.display());
    }
    assert!(seen >= 10, "the golden files are missing ({seen} found)");
}

#[test]
fn files_of_every_past_schema_version_still_load() {
    // One file per schema version that was ever written, kept as it was.
    // A new version adds the golden file of the version before to
    // `tests/fixtures` before blessing the new golden files.
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let mut seen = 0;
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let config =
            Config::from_json(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            config.desktop.schema_version,
            gevrek_model::settings::DESKTOP_CONFIG_SCHEMA_VERSION,
            "{}",
            path.display()
        );
        let errors: Vec<_> = config
            .validate()
            .into_iter()
            .filter(|issue| issue.severity == gevrek_model::settings::ConfigIssueSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
    }
    assert!(seen >= 2, "the fixtures are missing ({seen} found)");
}
