//! Zones that show what is somewhere else: a portal walking about in its
//! folder, a zone that fills itself from what the system knows (the
//! files used last, the programs that run), the filter typed into a
//! zone, and files made and moved in the folders that are shown.

use super::{Effect, Outcome, Pending, ZonesCore, ZonesEvent};
use crate::backend::{BackendError, BackendRequest, FileOperation};
use crate::config::{Matcher, SmartSource, ZoneDefinition, ZoneKind};
use crate::ids::{ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::paths;
use crate::portal::{self, Listing, PortalScene, PortalView};
use crate::rules;

/// A way for a portal to go.
pub(super) enum Way {
    /// Into a folder it shows.
    Into(ItemId),
    /// To a folder at or below its own.
    To(String),
    Up,
    Back,
    Forward,
}

impl ZonesCore {
    /// Where a portal stands, as far as it left its own folder.
    fn portal_view(&self, definition: &ZoneDefinition) -> Option<&PortalView> {
        let ZoneKind::Portal { directory } = &definition.kind else {
            return None;
        };
        self.view
            .portals
            .get(&definition.id)
            .filter(|view| view.root() == directory)
    }

    /// The folder a portal shows now: the one it walked to, or its own.
    /// Nothing for a zone that is no portal.
    pub(super) fn directory_of(&self, definition: &ZoneDefinition) -> Option<String> {
        let ZoneKind::Portal { directory } = &definition.kind else {
            return None;
        };
        Some(
            self.portal_view(definition)
                .map_or(directory.as_str(), |view| view.current())
                .to_string(),
        )
    }

    /// The folder a zone's items come from: a portal's, or the folder a
    /// zone that fills itself looks through.
    fn folder_of(&self, definition: &ZoneDefinition) -> Option<String> {
        match (&definition.kind, &definition.source) {
            (ZoneKind::Smart { .. }, SmartSource::Folder { directory }) => Some(directory.clone()),
            _ => self.directory_of(definition),
        }
    }

    /// The zones that show a folder.
    pub(super) fn showing(&self, directory: &str) -> Vec<ZoneId> {
        self.config
            .zones
            .iter()
            .filter(|zone| self.folder_of(zone).as_deref() == Some(directory))
            .map(|zone| zone.id.clone())
            .collect()
    }

    /// The portal that shows an item now, where the item is a folder: the
    /// portal can go into it.
    pub(super) fn portal_with_folder(&self, item: &ItemId) -> Option<ZoneId> {
        self.config.zones.iter().find_map(|zone| {
            let directory = self.directory_of(zone)?;
            let shown = self.directories.get(&directory)?.get(item)?;
            (shown.kind == ItemKind::Folder).then(|| zone.id.clone())
        })
    }

    pub(super) fn portal_scene(&self, definition: &ZoneDefinition) -> Option<PortalScene> {
        let ZoneKind::Portal { directory } = &definition.kind else {
            return None;
        };
        let fresh = PortalView::new(directory);
        let view = self.portal_view(definition).unwrap_or(&fresh);
        let listing = self
            .listings
            .get(view.current())
            .cloned()
            .unwrap_or_default();
        Some(view.scene(listing))
    }

    /// How far the listing has come of what a zone that fills itself
    /// looks through. Nothing for the desktop, which is always there.
    pub(super) fn source_listing(&self, definition: &ZoneDefinition) -> Option<Listing> {
        if !matches!(definition.kind, ZoneKind::Smart { .. }) {
            return None;
        }
        match &definition.source {
            SmartSource::Desktop => None,
            SmartSource::Folder { directory } => {
                Some(self.listings.get(directory).cloned().unwrap_or_default())
            }
            source => Some(
                self.source_listings
                    .get(source)
                    .cloned()
                    .unwrap_or_default(),
            ),
        }
    }

    /// What a zone that fills itself shows: everything its source has
    /// that all of its query holds for, in the source's order. Asked of
    /// the desktop, an empty query shows nothing (it would be the whole
    /// desktop again); asked of anything else, it shows all there is.
    pub(super) fn smart_items(
        &self,
        definition: &ZoneDefinition,
        query: &[Matcher],
        now: u64,
    ) -> Vec<&Item> {
        let fits = |item: &&Item| query.iter().all(|matcher| rules::holds(matcher, item, now));
        match &definition.source {
            SmartSource::Desktop if query.is_empty() => Vec::new(),
            SmartSource::Desktop => self.items.values().filter(fits).collect(),
            SmartSource::Folder { directory } => self
                .directories
                .get(directory)
                .map(|items| items.values().filter(fits).collect())
                .unwrap_or_default(),
            source => self
                .sources
                .get(source)
                .map(|items| items.iter().filter(fits).collect())
                .unwrap_or_default(),
        }
    }

    /// The listing of a folder stands otherwise: tell the zones that
    /// show it.
    pub(super) fn listing_is(&mut self, directory: &str, listing: Listing) -> Vec<Effect> {
        if self.listings.get(directory) == Some(&listing) {
            return Vec::new();
        }
        self.listings.insert(directory.to_string(), listing.clone());
        self.showing(directory)
            .into_iter()
            .map(|zone| {
                Effect::Notify(ZonesEvent::ListingChanged {
                    zone,
                    listing: listing.clone(),
                })
            })
            .collect()
    }

    /// The zones that fill themselves from a source.
    fn fed_by(&self, source: &SmartSource) -> Vec<ZoneId> {
        self.config
            .zones
            .iter()
            .filter(|zone| matches!(zone.kind, ZoneKind::Smart { .. }) && &zone.source == source)
            .map(|zone| zone.id.clone())
            .collect()
    }

    fn source_is(&mut self, source: &SmartSource, listing: Listing) -> Vec<Effect> {
        let changed = self.source_listings.get(source) != Some(&listing);
        self.source_listings.insert(source.clone(), listing.clone());
        let mut effects = Vec::new();
        for zone in self.fed_by(source) {
            if changed {
                effects.push(Effect::Notify(ZonesEvent::ListingChanged {
                    zone: zone.clone(),
                    listing: listing.clone(),
                }));
            }
            effects.push(Effect::Notify(ZonesEvent::ItemsChanged {
                zone: Some(zone),
            }));
        }
        effects
    }

    pub(super) fn source_listed(&mut self, source: SmartSource, items: Vec<Item>) -> Vec<Effect> {
        // (An answer for a source no zone looks through any more.)
        if !self.watched_sources.contains(&source) {
            return Vec::new();
        }
        self.sources.insert(source.clone(), items);
        self.source_is(&source, Listing::Ready)
    }

    pub(super) fn source_failed(
        &mut self,
        source: SmartSource,
        error: BackendError,
    ) -> Vec<Effect> {
        if !self.watched_sources.contains(&source) {
            return Vec::new();
        }
        log::debug!("the source {source:?} could not be asked: {error}");
        // A backend that cannot ask a source at all is not the one that
        // knows it: where whoever hosts the core told what it has (the
        // programs that run), that stands.
        if error == BackendError::Unsupported && self.sources.contains_key(&source) {
            return self.source_is(&source, Listing::Ready);
        }
        self.sources.remove(&source);
        self.source_is(
            &source,
            Listing::Unavailable {
                reason: error.to_string(),
            },
        )
    }

    pub(super) fn navigate(&mut self, zone: &ZoneId, way: Way) -> Outcome {
        let definition = self.known(zone)?.clone();
        let ZoneKind::Portal { directory: root } = &definition.kind else {
            return Err(format!("{zone} is no portal"));
        };
        let mut view = self
            .portal_view(&definition)
            .cloned()
            .unwrap_or_else(|| PortalView::new(root));
        let went = match way {
            Way::Into(item) => {
                let folder = self
                    .directories
                    .get(view.current())
                    .and_then(|items| items.get(&item))
                    .filter(|shown| shown.kind == ItemKind::Folder)
                    .and_then(|shown| shown.path.clone())
                    .ok_or_else(|| format!("{item} is no folder that {zone} shows"))?;
                view.go(&folder)
            }
            Way::To(directory) => {
                if !portal::within(root, &directory) {
                    return Err(format!("{directory} is not within the folder of {zone}"));
                }
                view.go(&directory)
            }
            Way::Up if view.at_root() => {
                return Err(format!("{zone} is in its own folder: there is no way up"))
            }
            Way::Up => view.up(),
            Way::Back if !view.can_go_back() => {
                return Err(format!("{zone} has not been anywhere else"))
            }
            Way::Back => view.back(),
            Way::Forward if !view.can_go_forward() => {
                return Err(format!("{zone} has nowhere to go forward to"))
            }
            Way::Forward => view.forward(),
        };
        if !went {
            return Ok(Vec::new());
        }
        let directory = view.current().to_string();
        self.view.portals.insert(zone.clone(), view);
        // What was typed was typed of the folder that was shown.
        let filtered = self.view.filters.remove(zone).is_some();
        let mut effects = vec![Effect::Notify(ZonesEvent::Navigated {
            zone: zone.clone(),
            directory,
        })];
        if filtered {
            effects.push(Effect::Notify(ZonesEvent::FilterChanged {
                zone: zone.clone(),
                typed: String::new(),
            }));
        }
        effects.extend(self.sync_watches());
        effects.push(Effect::Notify(ZonesEvent::ItemsChanged {
            zone: Some(zone.clone()),
        }));
        Ok(effects)
    }

    pub(super) fn refresh_zone(&mut self, zone: &ZoneId) -> Outcome {
        let definition = self.known(zone)?.clone();
        if let Some(directory) = self.folder_of(&definition) {
            let mut effects = self.listing_is(&directory, Listing::Loading);
            effects.push(Effect::Ask(BackendRequest::Relist { directory }));
            return Ok(effects);
        }
        match (&definition.kind, &definition.source) {
            (ZoneKind::Smart { .. }, source) if self.watched_sources.contains(source) => {
                let mut effects = self.source_is(source, Listing::Loading);
                effects.push(Effect::Ask(BackendRequest::ListSource {
                    source: source.clone(),
                }));
                Ok(effects)
            }
            // The desktop is told of as it changes: there is nothing to ask.
            _ => Ok(Vec::new()),
        }
    }

    pub(super) fn new_folder(&mut self, zone: &ZoneId, name: &str) -> Outcome {
        let definition = self.known(zone)?.clone();
        let parent = self
            .directory_of(&definition)
            .ok_or_else(|| format!("{zone} shows no folder to make one in"))?;
        self.new_folder_in(parent, name)
    }

    pub(super) fn new_folder_in(&mut self, parent: String, name: &str) -> Outcome {
        if parent.trim().is_empty() {
            return Err("name the folder the new one is made in".to_string());
        }
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err("that is no name for a folder".to_string());
        }
        let operation = FileOperation::NewFolder {
            parent,
            name: name.to_string(),
        };
        Ok(vec![self.ask(Pending::Portal { operation })])
    }

    pub(super) fn move_items_to_folder(
        &mut self,
        items: &[ItemId],
        folder: &str,
        copy: bool,
    ) -> Outcome {
        if folder.trim().is_empty() {
            return Err("name the folder the files go to".to_string());
        }
        let mut effects = Vec::new();
        for id in items {
            // Taking a file away takes it out of its zone, which a lock
            // forbids; a copy leaves it there.
            let locked = !copy
                && self
                    .zone_of(id)
                    .is_some_and(|zone| self.lock_of(zone).members);
            let Some(from) = self
                .item(id)
                .and_then(|item| item.path.clone())
                .filter(|_| !locked)
            else {
                continue;
            };
            if !copy && paths::parent(&from).is_some_and(|at| paths::same_folder(at, folder)) {
                continue;
            }
            // (A folder is not put into itself.)
            if portal::within(&from, folder) {
                continue;
            }
            let to_folder = folder.to_string();
            let operation = if copy {
                FileOperation::Copy { from, to_folder }
            } else {
                FileOperation::Move { from, to_folder }
            };
            effects.push(self.ask(Pending::Portal { operation }));
        }
        if effects.is_empty() {
            return Err("nothing of that can be put there".to_string());
        }
        Ok(effects)
    }

    /// The file operations that take long and have not ended, each with
    /// the zones it goes into or out of: the zone a file brought in is
    /// for, and the zones that show a folder it touches.
    pub(super) fn operations(&self) -> Vec<super::OperationScene> {
        self.view
            .long
            .iter()
            .filter_map(|(request, (done, total))| {
                let pending = self.pending.get(request)?;
                let operation = pending.operation();
                let (path, folders): (&str, Vec<&str>) = match operation {
                    FileOperation::Copy { from, to_folder }
                    | FileOperation::Move { from, to_folder } => (
                        from,
                        paths::parent(from)
                            .into_iter()
                            .chain([to_folder.as_str()])
                            .collect(),
                    ),
                    FileOperation::Rename { path, .. }
                    | FileOperation::Trash { path }
                    | FileOperation::Restore { path } => {
                        (path, paths::parent(path).into_iter().collect())
                    }
                    FileOperation::NewFolder { parent, .. } => (parent, vec![parent.as_str()]),
                };
                let item = self
                    .items
                    .values()
                    .chain(self.directories.values().flat_map(|items| items.values()))
                    .find(|item| item.path.as_deref() == Some(path))
                    .map(|item| item.id.clone());
                let mut zones: Vec<ZoneId> = folders
                    .iter()
                    .flat_map(|folder| self.showing(folder))
                    .collect();
                if let Pending::Arriving {
                    zone: Some(zone), ..
                } = pending
                {
                    zones.push(zone.clone());
                }
                zones.extend(item.as_ref().and_then(|item| self.zone_of(item)).cloned());
                zones.sort();
                zones.dedup();
                Some(super::OperationScene {
                    request: *request,
                    operation: operation.clone(),
                    done: *done,
                    total: *total,
                    zones,
                    item,
                })
            })
            .collect()
    }

    pub(super) fn filter(&mut self, zone: Option<ZoneId>, typed: &str) -> Outcome {
        let zone = match zone {
            Some(zone) => zone,
            None => match &self.view.focus {
                Some(focus) if focus.desk.is_some() => {
                    return Err("the desktop has the keyboard, not a zone".to_string());
                }
                Some(focus) => focus.zone.clone(),
                None => return Err("no zone has the keyboard".to_string()),
            },
        };
        self.known(&zone)?;
        let before = self.view.filters.get(&zone).cloned().unwrap_or_default();
        // (Kept as typed, spaces and all: it is shown back in the box.
        // Only what is nothing but spaces is no filter.)
        let typed = if typed.trim().is_empty() { "" } else { typed };
        if before == typed {
            return Ok(Vec::new());
        }
        if typed.is_empty() {
            self.view.filters.remove(&zone);
        } else {
            self.view.filters.insert(zone.clone(), typed.to_string());
        }
        Ok(vec![
            Effect::Notify(ZonesEvent::FilterChanged {
                zone: zone.clone(),
                typed: typed.to_string(),
            }),
            Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone) }),
        ])
    }
}
