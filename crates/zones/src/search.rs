//! Finding an item by typing some of its name, across every zone. The
//! marks are the launcher's (`gevrek_system::apps::score`), so the same
//! letters find things in the same order in both places: the whole name,
//! then its beginning, the beginning of a word in it, any part of it, and
//! last its letters in order with others between.

use crate::ids::{ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::layout::compare_names;
use serde::{Deserialize, Serialize};

/// How well a name fits what was typed; nothing when it does not fit.
pub fn score(name: &str, typed: &str) -> Option<u32> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Some(1);
    }
    let name = name.to_lowercase();
    if name == typed {
        return Some(1000);
    }
    if name.starts_with(&typed) {
        return Some(900 - name.chars().count().min(100) as u32);
    }
    if name
        .split(|letter: char| !letter.is_alphanumeric())
        .any(|word| word.starts_with(&typed))
    {
        return Some(700);
    }
    if name.contains(&typed) {
        return Some(600);
    }
    in_order(&name, &typed).then_some(300)
}

/// Whether the letters of `typed` are in `name` in their order, with
/// others between.
fn in_order(name: &str, typed: &str) -> bool {
    let mut letters = name.chars();
    typed
        .chars()
        .filter(|letter| !letter.is_whitespace())
        .all(|wanted| letters.any(|letter| letter == wanted))
}

/// One item that was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub item: ItemId,
    pub name: String,
    /// The zone it is shown in; none for a loose item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    pub score: u32,
    /// The zones that fill themselves and show it as well.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_in: Vec<ZoneId>,
    #[serde(default)]
    pub kind: ItemKind,
    /// Where it is in the file system, for what is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The items that fit what was typed, the best first; equally good ones
/// by name. Nothing typed finds nothing.
pub fn search<'a>(
    typed: &str,
    items: impl IntoIterator<Item = (&'a Item, Option<&'a ZoneId>)>,
) -> Vec<Hit> {
    if typed.trim().is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = items
        .into_iter()
        .filter_map(|(item, zone)| {
            Some(Hit {
                item: item.id.clone(),
                name: item.name.clone(),
                zone: zone.cloned(),
                score: score(&item.name, typed)?,
                also_in: Vec::new(),
                kind: item.kind,
                path: item.path.clone(),
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| compare_names(&a.name, &b.name))
            .then_with(|| a.item.cmp(&b.item))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_marked_by_how_it_fits() {
        for (name, typed, expected) in [
            ("Budget", "budget", Some(1000)),
            ("Budget 2026.xlsx", "bud", Some(900 - 16)),
            ("Annual Budget.xlsx", "bud", Some(700)),
            ("my-budget.xlsx", "bud", Some(700)),
            ("Rebudgeted.xlsx", "bud", Some(600)),
            ("Big ugly document", "bud", Some(300)),
            ("Notes.txt", "bud", None),
            ("Notes.txt", "  ", Some(1)),
            ("Überblick", "ÜBER", Some(900 - 9)),
        ] {
            assert_eq!(score(name, typed), expected, "{typed} in {name}");
        }
    }

    #[test]
    fn the_best_fit_is_first_whichever_zone_it_is_in() {
        let files: Vec<Item> = ["Reports", "report-10.pdf", "report-9.pdf", "Old report.pdf", "Photo.png"]
            .iter()
            .map(|name| Item::file(&format!("/desk/{name}")))
            .collect();
        let papers = ZoneId::new("papers");
        let hits = search(
            "report",
            files.iter().enumerate().map(|(index, item)| (item, (index % 2 == 0).then_some(&papers))),
        );
        let found: Vec<(&str, Option<&str>)> = hits
            .iter()
            .map(|hit| (hit.name.as_str(), hit.zone.as_ref().map(ZoneId::as_str)))
            .collect();
        assert_eq!(
            found,
            [
                ("Reports", Some("papers")),
                ("report-9.pdf", Some("papers")),
                ("report-10.pdf", None),
                ("Old report.pdf", None),
            ]
        );
        assert_eq!(search("", files.iter().map(|item| (item, None))), []);
    }
}
