//! What is known to be at a watched place, and what a change there means
//! to the core.
//!
//! Windows tells of changes loosely. The same file's coming is told
//! twice (once for the desktop of the shell, once for the folder it is
//! in), a file that is saved is told as deleted and made again by some
//! programs, and when much happens at once the system only says "look
//! again". The core wants each thing once: an item came, went, has
//! another name, changed. So what was last known to be at each watched
//! place is kept here, and every change is measured against it: a coming
//! of something that is there already is no coming, a going of something
//! unknown is nothing, and "look again" becomes exactly the differences.
//!
//! Nothing here asks the system: what changed is handed in, already read.

use gevrek_zones::backend::{BackendEvent, Location};
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::{Item, ItemKind};
use std::collections::BTreeMap;

/// Something that happened at a watched place.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// It is there (it came, or was written to).
    Came(Item),
    /// It is not there any more.
    Went(ItemId),
    /// What was `from` is now `item`. Without an item it has gone out of
    /// the place (renamed away, moved elsewhere).
    Renamed { from: ItemId, item: Option<Item> },
    /// Something about it is other than it was (the system said so, even
    /// where nothing that is read here differs: its picture, perhaps).
    Touched(Item),
    /// The pictures the shell draws changed (the Recycle Bin got its
    /// first thing, or its last went). Told for what is no file, whose
    /// picture is the only thing about it that can change.
    Pictures,
    /// Everything that is at the place now, read anew.
    Relisted(Vec<Item>),
}

/// Windows does not tell letters of one case from the other in names.
fn key(id: &ItemId) -> String {
    id.as_str().to_lowercase()
}

/// What is known to be at one watched place.
#[derive(Debug)]
pub struct Seen {
    location: Location,
    known: BTreeMap<String, Item>,
    /// While the place is being read whole: what changed meanwhile. The
    /// reading may have passed an item before it changed, or not.
    touched: Option<Vec<ItemId>>,
}

impl Seen {
    pub fn new(location: Location) -> Self {
        Self {
            location,
            known: BTreeMap::new(),
            touched: None,
        }
    }

    /// Whether an item of this id is known to be at the place.
    pub fn knows(&self, id: &ItemId) -> bool {
        self.known.contains_key(&key(id))
    }

    /// Everything that is known to be at the place.
    pub fn items(&self) -> Vec<Item> {
        self.known.values().cloned().collect()
    }

    /// The place is about to be read whole (which takes a while for a
    /// large folder): from now on it is noted which items change.
    pub fn begin_listing(&mut self) {
        self.touched = Some(Vec::new());
    }

    /// The reading is over: the items that changed while it went on. Each
    /// of them is to be read once more, since the listing may have it as
    /// it was before.
    pub fn end_listing(&mut self) -> Vec<ItemId> {
        self.touched.take().unwrap_or_default()
    }

    fn note(&mut self, change: &Change) {
        let Some(touched) = &mut self.touched else {
            return;
        };
        match change {
            Change::Came(item) | Change::Touched(item) => touched.push(item.id.clone()),
            Change::Went(id) => touched.push(id.clone()),
            Change::Renamed { from, item } => {
                touched.push(from.clone());
                touched.extend(item.iter().map(|item| item.id.clone()));
            }
            Change::Pictures | Change::Relisted(_) => {}
        }
    }

    pub fn location(&self) -> &Location {
        &self.location
    }

    /// The place was listed for the core: this is what the core knows now.
    pub fn listed(&mut self, items: &[Item]) {
        self.known = items
            .iter()
            .map(|item| (key(&item.id), item.clone()))
            .collect();
    }

    /// What a change is to the core, measured against what is known.
    pub fn apply(&mut self, change: Change) -> Vec<BackendEvent> {
        self.note(&change);
        let location = self.location.clone();
        match change {
            Change::Came(item) => match self.known.insert(key(&item.id), item.clone()) {
                None => vec![BackendEvent::ItemAdded { location, item }],
                Some(before) if before != item => {
                    vec![BackendEvent::ItemChanged { location, item }]
                }
                Some(_) => Vec::new(),
            },
            Change::Went(id) => match self.known.remove(&key(&id)) {
                Some(item) => vec![BackendEvent::ItemRemoved {
                    location,
                    item: item.id,
                }],
                None => Vec::new(),
            },
            Change::Renamed { from, item } => {
                let Some(before) = self.known.remove(&key(&from)) else {
                    // What was not known to be here came from elsewhere.
                    return item.map_or_else(Vec::new, |item| self.apply(Change::Came(item)));
                };
                let Some(item) = item else {
                    return vec![BackendEvent::ItemRemoved {
                        location,
                        item: before.id,
                    }];
                };
                match self.known.insert(key(&item.id), item.clone()) {
                    // It took the place of something that was here: that
                    // one is gone, and this one is what it is now.
                    Some(_) if key(&item.id) != key(&before.id) => vec![
                        BackendEvent::ItemRemoved {
                            location: location.clone(),
                            item: before.id,
                        },
                        BackendEvent::ItemChanged { location, item },
                    ],
                    _ => vec![BackendEvent::ItemRenamed {
                        location,
                        from: before.id,
                        item,
                    }],
                }
            }
            Change::Touched(item) => match self.known.insert(key(&item.id), item.clone()) {
                None => vec![BackendEvent::ItemAdded { location, item }],
                Some(_) => vec![BackendEvent::ItemChanged { location, item }],
            },
            Change::Pictures => self
                .known
                .values()
                .filter(|item| item.kind == ItemKind::Virtual)
                .map(|item| BackendEvent::ItemChanged {
                    location: location.clone(),
                    item: item.clone(),
                })
                .collect(),
            Change::Relisted(items) => {
                let now: BTreeMap<String, Item> = items
                    .into_iter()
                    .map(|item| (key(&item.id), item))
                    .collect();
                let mut events = Vec::new();
                for (key, before) in &self.known {
                    if !now.contains_key(key) {
                        events.push(BackendEvent::ItemRemoved {
                            location: location.clone(),
                            item: before.id.clone(),
                        });
                    }
                }
                for (key, item) in &now {
                    match self.known.get(key) {
                        None => events.push(BackendEvent::ItemAdded {
                            location: location.clone(),
                            item: item.clone(),
                        }),
                        Some(before) if before != item => events.push(BackendEvent::ItemChanged {
                            location: location.clone(),
                            item: item.clone(),
                        }),
                        Some(_) => {}
                    }
                }
                self.known = now;
                events
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESK: &str = "C:\\Users\\ada\\Desktop";

    fn file(name: &str) -> Item {
        Item::file(&format!("{DESK}\\{name}"))
    }

    fn sized(name: &str, size: u64) -> Item {
        let mut item = file(name);
        item.facts.size = Some(size);
        item
    }

    fn id(name: &str) -> ItemId {
        ItemId::new(format!("{DESK}\\{name}"))
    }

    fn seen(items: &[Item]) -> Seen {
        let mut seen = Seen::new(Location::Desktop);
        seen.listed(items);
        seen
    }

    const AT: Location = Location::Desktop;

    #[test]
    fn each_thing_is_told_once_however_often_windows_tells_it() {
        let mut seen = seen(&[file("a.txt")]);
        // The same coming told twice, as the shell does for the desktop.
        assert_eq!(
            seen.apply(Change::Came(file("b.txt"))),
            [BackendEvent::ItemAdded {
                location: AT,
                item: file("b.txt")
            }]
        );
        assert_eq!(seen.apply(Change::Came(file("b.txt"))), []);
        // Written to: told as a coming of what is there already.
        assert_eq!(
            seen.apply(Change::Came(sized("b.txt", 9))),
            [BackendEvent::ItemChanged {
                location: AT,
                item: sized("b.txt", 9)
            }]
        );
        // Going, told twice; and in letters of the other case, which
        // Windows does not tell apart. The id told is the one known.
        assert_eq!(
            seen.apply(Change::Went(ItemId::new(format!(
                "{}\\B.TXT",
                DESK.to_lowercase()
            )))),
            [BackendEvent::ItemRemoved {
                location: AT,
                item: id("b.txt")
            }]
        );
        assert_eq!(seen.apply(Change::Went(id("b.txt"))), []);
        assert_eq!(seen.apply(Change::Went(id("never.txt"))), []);
    }

    #[test]
    fn a_rename_keeps_the_item_and_knows_where_it_went() {
        let mut seen = seen(&[file("a.txt"), file("c.txt")]);
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("a.txt"),
                item: Some(file("b.txt"))
            }),
            [BackendEvent::ItemRenamed {
                location: AT,
                from: id("a.txt"),
                item: file("b.txt")
            }]
        );
        // Told again: the old name is not known any more, the new one is.
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("a.txt"),
                item: Some(file("b.txt"))
            }),
            []
        );
        // Only the letters' case.
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("b.txt"),
                item: Some(file("B.txt"))
            }),
            [BackendEvent::ItemRenamed {
                location: AT,
                from: id("b.txt"),
                item: file("B.txt")
            }]
        );
        // Onto another's name: that one is gone.
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("B.txt"),
                item: Some(file("c.txt"))
            }),
            [
                BackendEvent::ItemRemoved {
                    location: AT,
                    item: id("B.txt")
                },
                BackendEvent::ItemChanged {
                    location: AT,
                    item: file("c.txt")
                },
            ]
        );
        // Out of the place, and into it from elsewhere.
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("c.txt"),
                item: None
            }),
            [BackendEvent::ItemRemoved {
                location: AT,
                item: id("c.txt")
            }]
        );
        assert_eq!(
            seen.apply(Change::Renamed {
                from: ItemId::new("D:\\elsewhere\\x.txt"),
                item: Some(file("x.txt"))
            }),
            [BackendEvent::ItemAdded {
                location: AT,
                item: file("x.txt")
            }]
        );
        assert_eq!(
            seen.apply(Change::Renamed {
                from: id("gone.txt"),
                item: None
            }),
            []
        );
    }

    #[test]
    fn looking_again_tells_exactly_what_differs() {
        let mut seen = seen(&[file("a.txt"), file("b.txt"), file("c.txt")]);
        let events = seen.apply(Change::Relisted(vec![
            file("a.txt"),
            sized("b.txt", 4),
            file("d.txt"),
        ]));
        assert_eq!(
            events,
            [
                BackendEvent::ItemRemoved {
                    location: AT,
                    item: id("c.txt")
                },
                BackendEvent::ItemChanged {
                    location: AT,
                    item: sized("b.txt", 4)
                },
                BackendEvent::ItemAdded {
                    location: AT,
                    item: file("d.txt")
                },
            ]
        );
        assert_eq!(
            seen.apply(Change::Relisted(vec![
                file("a.txt"),
                sized("b.txt", 4),
                file("d.txt")
            ])),
            []
        );
    }

    #[test]
    fn a_touch_is_always_told_and_new_pictures_only_for_what_is_no_file() {
        let bin = Item {
            id: ItemId::new("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            kind: ItemKind::Virtual,
            name: "Recycle Bin".to_string(),
            ..Item::default()
        };
        let mut seen = seen(&[file("a.txt"), bin.clone()]);
        assert_eq!(
            seen.apply(Change::Touched(file("a.txt"))),
            [BackendEvent::ItemChanged {
                location: AT,
                item: file("a.txt")
            }]
        );
        assert_eq!(
            seen.apply(Change::Touched(file("new.txt"))),
            [BackendEvent::ItemAdded {
                location: AT,
                item: file("new.txt")
            }]
        );
        assert_eq!(
            seen.apply(Change::Pictures),
            [BackendEvent::ItemChanged {
                location: AT,
                item: bin
            }]
        );
    }
}
