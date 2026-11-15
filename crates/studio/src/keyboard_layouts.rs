//! The keyboard's layouts, chosen by their names: on the Input page.
//!
//! `desktop.input.keyboard.layouts` holds XKB's short names (`tr` with
//! the variant `f`). Nobody knows those by heart, so the card lists what
//! the system's XKB has, by what each is called ("Turkish (F)"), to be
//! searched and added; the chosen ones are put in order, given a
//! variant, or taken out. The first is the one a session starts with.
//!
//! The names are read from XKB's own list (`rules/evdev.lst`); where
//! there is none (Windows, macOS: the system's own keyboard settings
//! hold there, and the file's are kept for a Linux machine) a short list
//! that is built in stands for it.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_ui::{icons, theme, widgets};
use serde_json::{json, Value};
use std::sync::OnceLock;

const LAYOUTS: &str = "/desktop/input/keyboard/layouts";

/// What XKB has: its layouts and each one's variants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Known {
    /// A layout's name and what it is called.
    pub layouts: Vec<(String, String)>,
    /// The layout a variant is of, its name, and what it is called.
    pub variants: Vec<(String, String, String)>,
}

impl Known {
    /// What a layout is called (its name where XKB's list does not have it).
    pub fn called(&self, layout: &str, variant: &str) -> String {
        if !variant.is_empty() {
            if let Some((_, _, called)) = self
                .variants
                .iter()
                .find(|(of, name, _)| of == layout && name == variant)
            {
                return called.clone();
            }
        }
        let plain = self
            .layouts
            .iter()
            .find(|(name, _)| name == layout)
            .map_or_else(|| layout.to_string(), |(_, called)| called.clone());
        if variant.is_empty() {
            plain
        } else {
            format!("{plain} ({variant})")
        }
    }

    /// The variants of a layout: name and what it is called.
    pub fn variants_of<'a>(&'a self, layout: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.variants
            .iter()
            .filter(move |(of, _, _)| of == layout)
            .map(|(_, name, called)| (name.as_str(), called.as_str()))
    }

    /// The layouts and variants that what was typed finds, the plain
    /// layouts first: `(layout, variant, called)`. Nothing typed: every
    /// plain layout.
    pub fn found(&self, typed: &str, most: usize) -> Vec<(String, String, String)> {
        let typed = typed.trim().to_lowercase();
        let holds = |name: &str, called: &str| {
            typed.is_empty()
                || name.to_lowercase() == typed
                || called.to_lowercase().contains(&typed)
        };
        let mut found: Vec<(String, String, String)> = self
            .layouts
            .iter()
            .filter(|(name, called)| holds(name, called))
            .map(|(name, called)| (name.clone(), String::new(), called.clone()))
            .collect();
        if !typed.is_empty() {
            found.extend(
                self.variants
                    .iter()
                    .filter(|(_, _, called)| called.to_lowercase().contains(&typed))
                    .cloned(),
            );
        }
        found.truncate(most);
        found
    }
}

/// XKB's list as it is written: sections `! layout` and `! variant`, a
/// line a name and what it is called; a variant's begins with its layout
/// and a colon (`f  tr: Turkish (F)`).
pub fn parse(text: &str) -> Known {
    let mut known = Known::default();
    let mut section = "";
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('!') {
            section = name.trim();
            continue;
        }
        let Some((name, rest)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim();
        match section {
            "layout" => known.layouts.push((name.to_string(), rest.to_string())),
            "variant" => {
                if let Some((of, called)) = rest.split_once(':') {
                    known.variants.push((
                        of.trim().to_string(),
                        name.to_string(),
                        called.trim().to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
    known.layouts.sort_by_key(|(_, called)| called.to_lowercase());
    known
}

/// What stands for XKB's list where there is none.
const BUILT_IN: &str = "! layout
  us English (US)
  gb English (UK)
  de German
  fr French
  es Spanish
  it Italian
  pt Portuguese
  br Portuguese (Brazil)
  nl Dutch
  se Swedish
  no Norwegian
  dk Danish
  fi Finnish
  pl Polish
  cz Czech
  hu Hungarian
  ro Romanian
  gr Greek
  tr Turkish
  ru Russian
  ua Ukrainian
  il Hebrew
  ara Arabic
  jp Japanese
  kr Korean
  cn Chinese
! variant
  f tr: Turkish (F)
  dvorak us: English (Dvorak)
  colemak us: English (Colemak)
  intl us: English (US, intl., with dead keys)
  nodeadkeys de: German (no dead keys)
";

/// What this machine's XKB has, read once.
pub fn known() -> &'static Known {
    static KNOWN: OnceLock<Known> = OnceLock::new();
    KNOWN.get_or_init(|| {
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        if let Some(root) = std::env::var_os("XKB_CONFIG_ROOT") {
            roots.push(root.into());
        }
        roots.push("/usr/share/X11/xkb".into());
        roots.push("/usr/share/xkeyboard-config-2".into());
        roots
            .iter()
            .flat_map(|root| ["evdev.lst", "base.lst"].map(|name| root.join("rules").join(name)))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse(&text))
            .find(|known| !known.layouts.is_empty())
            .unwrap_or_else(|| parse(BUILT_IN))
    })
}

/// The chosen layouts as the configuration has them: `(layout, variant)`.
pub fn chosen(document: &Document) -> Vec<(String, String)> {
    document
        .value()
        .pointer(LAYOUTS)
        .and_then(Value::as_array)
        .map(|layouts| {
            layouts
                .iter()
                .map(|layout| {
                    let text = |key: &str| layout[key].as_str().unwrap_or_default().to_string();
                    (text("layout"), text("variant"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What is done to the list.
enum Done {
    Add(String, String),
    Remove(usize),
    Up(usize),
    Variant(usize, String),
}

/// The card's body.
pub fn show(ui: &mut Ui, document: &mut Document) {
    let palette = theme::pal(ui.ctx());
    let known = known();
    let chosen = chosen(document);
    let mut done: Option<Done> = None;

    if cfg!(not(target_os = "linux")) {
        ui.label(
            RichText::new(
                "This system keeps the keyboard's layouts itself; what is chosen here is \
                 written to the file and holds where gevrek is the session (Linux).",
            )
            .color(palette.warn),
        );
    }
    if chosen.is_empty() {
        ui.label(
            RichText::new("None chosen: the layout is what the system says (else English, US).")
                .color(palette.muted),
        );
    }
    for (index, (layout, variant)) in chosen.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{}.", index + 1))
                    .monospace()
                    .color(palette.faint),
            );
            ui.label(RichText::new(known.called(layout, "")).color(palette.text));
            ui.label(
                RichText::new(layout)
                    .monospace()
                    .size(12.0)
                    .color(palette.muted),
            );
            // Its variant, of those XKB has for it.
            let variants: Vec<(&str, &str)> = known.variants_of(layout).collect();
            if !variants.is_empty() || !variant.is_empty() {
                let shown = if variant.is_empty() {
                    "plain".to_string()
                } else {
                    known.called(layout, variant)
                };
                egui::ComboBox::from_id_salt(("a layout's variant", index))
                    .selected_text(shown)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(variant.is_empty(), "plain").clicked() {
                            done = Some(Done::Variant(index, String::new()));
                        }
                        for (name, called) in variants {
                            if ui.selectable_label(name == variant, called).clicked() {
                                done = Some(Done::Variant(index, name.to_string()));
                            }
                        }
                    });
            }
            if index == 0 {
                ui.label(RichText::new("the first").size(11.5).color(palette.faint));
            } else if widgets::small_icon_button(ui, icons::ARROW_UP, "Before the one above")
                .clicked()
            {
                done = Some(Done::Up(index));
            }
            if widgets::small_icon_button(ui, icons::TRASH, "Take it out").clicked() {
                done = Some(Done::Remove(index));
            }
        });
    }

    ui.add_space(6.0);
    let search = egui::Id::unique("a keyboard layout searched for");
    let mut typed: String = ui.data(|data| data.get_temp(search)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(icons::MAGNIFYING_GLASS).color(palette.muted));
        ui.add(
            egui::TextEdit::singleline(&mut typed)
                .hint_text("Add a layout: a language, a country, a name")
                .desired_width(280.0),
        );
    });
    if !typed.trim().is_empty() {
        let found = known.found(&typed, 12);
        if found.is_empty() {
            ui.label(RichText::new("XKB has no layout called so.").color(palette.muted));
        }
        for (layout, variant, called) in found {
            let has = chosen
                .iter()
                .any(|(has_layout, has_variant)| *has_layout == layout && *has_variant == variant);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!has, egui::Button::new(icons::PLUS).small())
                    .on_hover_text("Add it")
                    .clicked()
                {
                    done = Some(Done::Add(layout.clone(), variant.clone()));
                }
                ui.label(RichText::new(&called).color(if has {
                    palette.muted
                } else {
                    palette.text
                }));
                let short = if variant.is_empty() {
                    layout.clone()
                } else {
                    format!("{layout} {variant}")
                };
                ui.label(
                    RichText::new(short)
                        .monospace()
                        .size(12.0)
                        .color(palette.muted),
                );
                if has {
                    ui.label(RichText::new("chosen").size(11.5).color(palette.faint));
                }
            });
        }
    }
    if matches!(done, Some(Done::Add(..))) {
        typed.clear();
    }
    ui.data_mut(|data| data.insert_temp(search, typed));

    let Some(done) = done else { return };
    if document.get(LAYOUTS).is_none() {
        document.set(LAYOUTS, json!([]));
    }
    document.edit(LAYOUTS, |layouts| {
        let Some(layouts) = layouts.as_array_mut() else {
            return;
        };
        apply(layouts, done);
    });
}

fn apply(layouts: &mut Vec<Value>, done: Done) {
    match done {
        Done::Add(layout, variant) => layouts.push(json!({"layout": layout, "variant": variant})),
        Done::Remove(index) if index < layouts.len() => {
            layouts.remove(index);
        }
        Done::Up(index) if index > 0 && index < layouts.len() => layouts.swap(index, index - 1),
        Done::Variant(index, variant) => {
            if let Some(Value::Object(layout)) = layouts.get_mut(index) {
                layout.insert("variant".to_string(), Value::String(variant));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "! model\n  pc105 Generic 105-key PC\n! layout\n  us              English (US)\n  tr              Turkish\n  de              German\n! variant\n  f               tr: Turkish (F)\n  dvorak          us: English (Dvorak)\n! option\n  grp Switching to another layout\n";

    #[test]
    fn xkbs_list_is_read_by_its_sections() {
        let known = parse(LIST);
        // By what they are called, in order; a model is no layout.
        assert_eq!(
            known.layouts,
            [("us", "English (US)"), ("de", "German"), ("tr", "Turkish")]
                .map(|(name, called)| (name.to_string(), called.to_string()))
        );
        assert_eq!(known.variants.len(), 2);
        assert_eq!(known.called("tr", ""), "Turkish");
        assert_eq!(known.called("tr", "f"), "Turkish (F)");
        // Ones the list does not have are called by their names.
        assert_eq!(known.called("xx", ""), "xx");
        assert_eq!(known.called("tr", "q"), "Turkish (q)");
        assert_eq!(
            known.variants_of("us").collect::<Vec<_>>(),
            [("dvorak", "English (Dvorak)")]
        );
        // What is built in reads the same way.
        let built_in = parse(BUILT_IN);
        assert!(built_in.layouts.len() > 20 && built_in.called("tr", "f") == "Turkish (F)");
    }

    #[test]
    fn a_layout_is_found_by_what_it_is_called_or_its_name() {
        let known = parse(LIST);
        let names = |typed: &str| -> Vec<(String, String)> {
            known
                .found(typed, 10)
                .into_iter()
                .map(|(layout, variant, _)| (layout, variant))
                .collect()
        };
        let pair = |layout: &str, variant: &str| (layout.to_string(), variant.to_string());
        assert_eq!(names("turk"), [pair("tr", ""), pair("tr", "f")]);
        assert_eq!(names("DE"), [pair("de", "")]);
        assert_eq!(names("dvorak"), [pair("us", "dvorak")]);
        assert_eq!(names("").len(), 3);
        assert!(names("klingon").is_empty());
        assert_eq!(known.found("e", 1).len(), 1);
    }

    #[test]
    fn the_list_is_added_to_put_in_order_and_taken_from() {
        let mut layouts = Vec::new();
        apply(&mut layouts, Done::Add("us".into(), String::new()));
        apply(&mut layouts, Done::Add("tr".into(), String::new()));
        apply(&mut layouts, Done::Variant(1, "f".into()));
        assert_eq!(
            Value::Array(layouts.clone()),
            json!([{"layout": "us", "variant": ""}, {"layout": "tr", "variant": "f"}])
        );
        apply(&mut layouts, Done::Up(1));
        assert_eq!(layouts[0]["layout"], "tr");
        // The first cannot go higher; a place that is not there changes nothing.
        apply(&mut layouts, Done::Up(0));
        apply(&mut layouts, Done::Remove(7));
        assert_eq!(layouts.len(), 2);
        apply(&mut layouts, Done::Remove(0));
        assert_eq!(
            Value::Array(layouts),
            json!([{"layout": "us", "variant": ""}])
        );
    }
}
