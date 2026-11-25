//! Whole layouts: saved copies and what is done with them, a layout
//! carried in from elsewhere, an arrangement taken over from another set
//! of displays, and changes to the configuration that come as a patch.
//!
//! One thing holds for all of it: a layout is never lost. Whatever puts
//! another whole layout in the place of the one there is (a snapshot
//! restored, a document brought in) keeps the one there was as a
//! snapshot first, unless a snapshot already holds exactly that.

use super::{Effect, Outcome, ZonesCore, ZonesEvent};
use crate::config::{ZoneDefinition, ZonesConfig};
use crate::export::{self, LayoutDocument};
use crate::ids::ZoneId;
use crate::profile::ProfileKey;
use crate::state::{Snapshot, AUTOMATIC_KEPT};
use serde_json::Value;

/// Lay `changes` over `target` as a JSON merge patch (RFC 7386): an
/// object is gone into key by key, `null` takes a key away, and anything
/// else replaces what was there.
pub fn merge(target: &mut Value, changes: &Value) {
    let Value::Object(changes) = changes else {
        *target = changes.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let Value::Object(target) = target else {
        return;
    };
    for (key, change) in changes {
        if change.is_null() {
            target.remove(key);
        } else {
            merge(target.entry(key.clone()).or_insert(Value::Null), change);
        }
    }
}

impl ZonesCore {
    /// The layout as it is now, as a copy that can be kept.
    pub fn current_layout(&self, name: &str, now: u64) -> Snapshot {
        Snapshot {
            name: name.to_string(),
            taken_at: now,
            zones: self.config.zones.clone(),
            rules: self.config.rules.clone(),
            catch_all: self.config.catch_all.clone(),
            placements: self.state.placements.clone(),
            profiles: self.state.profiles.clone(),
            active_tabs: self.state.active_tabs.clone(),
            automatic: false,
            extra: Default::default(),
        }
    }

    /// A name no snapshot has: the one wanted, or that with a number.
    fn free_name(&self, wanted: &str) -> String {
        let taken = |name: &str| {
            self.state
                .snapshots
                .iter()
                .any(|snapshot| snapshot.name == name)
        };
        if !taken(wanted) {
            return wanted.to_string();
        }
        (2u32..)
            .map(|number| format!("{wanted} ({number})"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| wanted.to_string())
    }

    /// Keep the layout there is before something replaces it, and say
    /// under which name it can be found. Where a snapshot holds exactly
    /// this layout already, that one is named and nothing is added. Of
    /// the copies taken here only the last few are kept; the ones a
    /// person saved are never touched.
    fn keep_what_there_is(&mut self, before: &str, now: u64) -> String {
        let current = self.current_layout("", now);
        if let Some(same) = self
            .state
            .snapshots
            .iter()
            .find(|snapshot| snapshot.same_layout(&current))
        {
            return same.name.clone();
        }
        let name = self.free_name(&format!("Before {before}"));
        self.state.snapshots.push(Snapshot {
            name: name.clone(),
            automatic: true,
            ..current
        });
        let automatic = self
            .state
            .snapshots
            .iter()
            .filter(|snapshot| snapshot.automatic)
            .count();
        let mut to_drop = automatic.saturating_sub(AUTOMATIC_KEPT);
        self.state.snapshots.retain(|snapshot| {
            let drop = snapshot.automatic && to_drop > 0;
            if drop {
                to_drop -= 1;
            }
            !drop
        });
        name
    }

    /// Put a whole layout in force.
    fn put_layout(&mut self, layout: Snapshot) -> Vec<Effect> {
        self.config.zones = layout.zones;
        self.config.rules = layout.rules;
        self.config.catch_all = layout.catch_all;
        self.state.placements = layout.placements;
        self.state.profiles = layout.profiles;
        self.state.active_tabs = layout.active_tabs;
        // What the rules did before no longer describes what is there,
        // nor what making zones by type did.
        self.state.undo = None;
        self.state.auto_undo = None;
        self.current = None;
        let mut effects = self.sync_watches();
        self.choose_profile();
        self.reconcile();
        effects.extend([
            Effect::Notify(ZonesEvent::LayoutChanged),
            Effect::SaveConfig,
            Effect::SaveState,
        ]);
        effects
    }

    pub(super) fn save_snapshot(&mut self, name: String, now: u64) -> Outcome {
        if name.trim().is_empty() {
            return Err("a snapshot needs a name".to_string());
        }
        let snapshot = self.current_layout(&name, now);
        match self
            .state
            .snapshots
            .iter_mut()
            .find(|snapshot| snapshot.name == name)
        {
            Some(existing) => *existing = snapshot,
            None => self.state.snapshots.push(snapshot),
        }
        Ok(vec![
            Effect::Notify(ZonesEvent::SnapshotSaved { name }),
            Effect::SaveState,
        ])
    }

    pub(super) fn restore_snapshot(&mut self, name: &str, now: u64) -> Outcome {
        let snapshot = self
            .state
            .snapshots
            .iter()
            .find(|snapshot| snapshot.name == name)
            .cloned()
            .ok_or_else(|| format!("no snapshot is called {name}"))?;
        self.keep_what_there_is("restoring", now);
        let mut effects = vec![Effect::Notify(ZonesEvent::SnapshotRestored {
            name: name.to_string(),
        })];
        effects.extend(self.put_layout(snapshot));
        Ok(effects)
    }

    pub(super) fn rename_snapshot(&mut self, name: &str, to: &str) -> Outcome {
        if to.trim().is_empty() {
            return Err("a snapshot needs a name".to_string());
        }
        if name == to {
            return Ok(Vec::new());
        }
        if self
            .state
            .snapshots
            .iter()
            .any(|snapshot| snapshot.name == to)
        {
            return Err(format!("a snapshot is called {to} already"));
        }
        let snapshot = self
            .state
            .snapshots
            .iter_mut()
            .find(|snapshot| snapshot.name == name)
            .ok_or_else(|| format!("no snapshot is called {name}"))?;
        snapshot.name = to.to_string();
        // One a person named is theirs to keep.
        snapshot.automatic = false;
        Ok(vec![
            Effect::Notify(ZonesEvent::SnapshotRenamed {
                name: name.to_string(),
                to: to.to_string(),
            }),
            Effect::SaveState,
        ])
    }

    pub(super) fn export_layout(&mut self, name: Option<&str>, now: u64) -> Outcome {
        let layout = match name {
            Some(name) => self
                .state
                .snapshots
                .iter()
                .find(|snapshot| snapshot.name == name)
                .cloned()
                .ok_or_else(|| format!("no snapshot is called {name}"))?,
            None => self.current_layout("Layout", now),
        };
        let document = Box::new(LayoutDocument::of(layout, now));
        Ok(vec![Effect::Notify(ZonesEvent::LayoutExported {
            document,
        })])
    }

    pub(super) fn import_layout(&mut self, document: &Value, apply: bool, now: u64) -> Outcome {
        // Looked over before anything is touched: a document that cannot
        // be right changes nothing.
        let read = export::read(document)?;
        let kept_as = apply.then(|| self.keep_what_there_is("import", now));
        // (Under a name of its own: taking one in never replaces a
        // snapshot that is there.)
        let name = self.free_name(&read.layout.name);
        let layout = Snapshot {
            name: name.clone(),
            automatic: false,
            ..read.layout
        };
        self.state.snapshots.push(layout.clone());
        let mut effects = vec![Effect::Notify(ZonesEvent::LayoutImported {
            name,
            applied: apply,
            kept_as,
        })];
        if apply {
            effects.extend(self.put_layout(layout));
        } else {
            effects.push(Effect::SaveState);
        }
        Ok(effects)
    }

    pub(super) fn copy_arrangement(&mut self, from: &ProfileKey) -> Outcome {
        if self.config.lock.geometry {
            return Err("the zones are locked where they are".to_string());
        }
        let source = self
            .state
            .profiles
            .iter()
            .find(|profile| &profile.key == from)
            .cloned()
            .ok_or("no arrangement is kept for those displays")?;
        let index = self.current.ok_or("no display is known yet")?;
        if &self.state.profiles[index].key == from {
            return Ok(Vec::new());
        }
        let known: Vec<ZoneId> = self
            .config
            .zones
            .iter()
            .map(|zone| zone.id.clone())
            .collect();
        let profile = &mut self.state.profiles[index];
        for zone in &known {
            if let Some(place) = source.geometry.get(zone) {
                profile.geometry.insert(zone.clone(), place.clone());
            }
        }
        profile.hidden = source
            .hidden
            .iter()
            .filter(|zone| known.contains(zone))
            .cloned()
            .collect();
        // Zones that were on a display that is not here come to one that is.
        self.reconcile();
        Ok(vec![
            Effect::Notify(ZonesEvent::LayoutChanged),
            Effect::SaveState,
        ])
    }

    // ---- the configuration, changed by a patch ------------------------

    pub(super) fn modify_zone(&mut self, zone: &ZoneId, changes: &Value) -> Outcome {
        let before = self.known(zone)?.clone();
        let index = self
            .config
            .zones
            .iter()
            .position(|definition| &definition.id == zone)
            .unwrap_or(0);
        if !changes.is_object() {
            return Err("the changes to a zone are an object of its keys".to_string());
        }
        let mut written = serde_json::to_value(&before).map_err(|error| error.to_string())?;
        merge(&mut written, changes);
        let after: ZoneDefinition =
            serde_json::from_value(written).map_err(|error| format!("that is no zone: {error}"))?;
        if after.id != before.id {
            return Err("a zone keeps its id".to_string());
        }
        if after == before {
            return Ok(Vec::new());
        }
        let mut tried = self.config.clone();
        tried.zones[index] = after.clone();
        let place = format!("desktop.zones.zones[{index}]");
        if let Some((path, message)) = tried
            .problems()
            .into_iter()
            .find(|(path, _)| path.starts_with(&place))
        {
            let key = path
                .strip_prefix(&place)
                .unwrap_or(&path)
                .trim_start_matches('.');
            return Err(format!("{key}: {message}"));
        }
        let group = before.tab_group.clone();
        self.config.zones[index] = after;
        if group != self.config.zones[index].tab_group {
            self.dissolve_lone_tab(group);
        }
        let mut effects = vec![Effect::Notify(ZonesEvent::ZoneChanged {
            zone: zone.clone(),
        })];
        effects.extend(self.sync_watches());
        if self.reconcile() {
            effects.push(Effect::SaveState);
        }
        effects.extend([
            Effect::Notify(ZonesEvent::ItemsChanged {
                zone: Some(zone.clone()),
            }),
            Effect::Notify(ZonesEvent::LayoutChanged),
            Effect::SaveConfig,
        ]);
        Ok(effects)
    }

    pub(super) fn modify_settings(&mut self, changes: &Value) -> Outcome {
        let Some(keys) = changes.as_object() else {
            return Err("the changes to the settings are an object of their keys".to_string());
        };
        if keys.contains_key("zones") {
            return Err(
                "the zones are changed one by one (modify_zone), not with the settings".to_string(),
            );
        }
        let mut written = serde_json::to_value(&self.config).map_err(|error| error.to_string())?;
        merge(&mut written, changes);
        let after: ZonesConfig = serde_json::from_value(written)
            .map_err(|error| format!("those are no settings: {error}"))?;
        if let Some((path, message)) = after.problems().into_iter().next() {
            let key = path.strip_prefix("desktop.zones.").unwrap_or(&path);
            return Err(format!("{key}: {message}"));
        }
        if after == self.config {
            return Ok(Vec::new());
        }
        let mut effects = vec![Effect::Notify(ZonesEvent::SettingsChanged)];
        effects.extend(self.take_config(after));
        effects.push(Effect::SaveConfig);
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::merge;
    use serde_json::json;

    #[test]
    fn a_patch_is_laid_over_key_by_key() {
        let mut target =
            json!({"title": "Papers", "look": {"opacity": 60, "blur": true}, "private": true});
        merge(
            &mut target,
            &json!({"look": {"opacity": 80, "blur": null}, "private": null, "limit": 5}),
        );
        assert_eq!(
            target,
            json!({"title": "Papers", "look": {"opacity": 80}, "limit": 5})
        );
        // What is no object replaces what was there, whatever that was.
        merge(
            &mut target,
            &json!({"look": "plain", "title": {"long": "Papers", "short": null}}),
        );
        assert_eq!(
            target,
            json!({"title": {"long": "Papers"}, "look": "plain", "limit": 5})
        );
    }
}
