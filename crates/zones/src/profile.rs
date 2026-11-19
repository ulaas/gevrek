//! One arrangement of the zones for each set of displays. The set is
//! known by the displays' ids, sizes and scales (and by the workspace,
//! where each workspace has its own), in no order: plugging the same
//! displays into other ports changes nothing.
//!
//! When the displays are a set never seen before, an arrangement is made
//! for it from the nearest one there is. Zones of a display that is not
//! there go to one that is, holding on to the same corner. The
//! arrangement that was left is kept as it was, so when the old set is
//! back, every zone is back where it was.

use crate::geometry::{fallback_display, Display, ZoneGeometry};
use crate::ids::{DisplayId, ZoneId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How many arrangements are kept. A set of displays is a new one with
/// every other resolution or scale of one of them, so their number grows
/// for as long as the machine is used; beyond this many, the one that
/// was used longest ago is forgotten.
pub const MOST: usize = 48;

/// One display of a set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DisplayKey {
    pub id: DisplayId,
    /// In logical units.
    pub width: i32,
    pub height: i32,
    /// In thousandths (1500 for one and a half).
    pub scale: u32,
}

impl DisplayKey {
    pub fn of(display: &Display) -> Self {
        Self {
            id: display.id.clone(),
            width: display.rect.width,
            height: display.rect.height,
            scale: (display.scale.clamp(0.0, 1000.0) * 1000.0).round() as u32,
        }
    }
}

/// What an arrangement is for.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProfileKey {
    /// Ordered by id, each display once.
    pub displays: Vec<DisplayKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

impl ProfileKey {
    pub fn of(displays: &[Display], workspace: Option<&str>) -> Self {
        let mut keys: Vec<DisplayKey> = displays.iter().map(DisplayKey::of).collect();
        keys.sort();
        keys.dedup_by(|later, earlier| later.id == earlier.id);
        Self {
            displays: keys,
            workspace: workspace.map(str::to_string),
        }
    }

    fn has(&self, id: &DisplayId) -> bool {
        self.displays.iter().any(|display| &display.id == id)
    }
}

/// One arrangement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub key: ProfileKey,
    pub geometry: BTreeMap<ZoneId, ZoneGeometry>,
    /// The zones that are not shown here. (The hidden ones are named, not
    /// the shown: a zone made later is shown everywhere.)
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden: BTreeSet<ZoneId>,
    /// A count that grows with every choice, for telling which
    /// arrangement was used last. Not a time: nothing here reads a clock.
    pub last_used: u64,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: gevrek_model::Extra,
}

/// What choosing an arrangement gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    /// Its place in the list.
    pub index: usize,
    /// It was made now, from the arrangement with this key (or from
    /// nothing, when there was none).
    pub made: Option<Option<ProfileKey>>,
    /// The zones that were on a display that is not there and went to
    /// another.
    pub moved: Vec<ZoneId>,
}

/// How near an arrangement is to a set of displays; more is nearer.
/// Displays in common count most (their zones are where they were), then
/// displays that are also as large as they were, then having nothing that
/// is not there, then the workspace, then what was used last.
fn nearness(candidate: &Profile, wanted: &ProfileKey) -> (usize, usize, bool, bool, u64) {
    let shared = wanted.displays.iter().filter(|display| candidate.key.has(&display.id)).count();
    let same = wanted
        .displays
        .iter()
        .filter(|display| candidate.key.displays.contains(display))
        .count();
    let nothing_missing = candidate.key.displays.iter().all(|display| wanted.has(&display.id));
    (
        shared,
        same,
        nothing_missing,
        candidate.key.workspace == wanted.workspace,
        candidate.last_used,
    )
}

/// The arrangement nearest to a key.
pub fn nearest(profiles: &[Profile], wanted: &ProfileKey) -> Option<usize> {
    profiles
        .iter()
        .enumerate()
        .max_by_key(|(_, profile)| nearness(profile, wanted))
        .map(|(index, _)| index)
}

/// Send the zones of displays that are not there to one that is. Each
/// keeps its corner, its distance from it and its size. Returns the zones
/// that went.
pub fn rehome(profile: &mut Profile, displays: &[Display]) -> Vec<ZoneId> {
    let Some(fallback) = fallback_display(displays) else {
        return Vec::new();
    };
    let mut moved = Vec::new();
    for (zone, geometry) in &mut profile.geometry {
        if !displays.iter().any(|display| display.id == geometry.display) {
            geometry.display = fallback.id.clone();
            moved.push(zone.clone());
        }
    }
    moved
}

/// The arrangement for the displays as they are now: the one made for
/// exactly this set, or a new one made from the nearest. `count` is the
/// number the chosen one is marked with as the last used. Nothing is
/// chosen while there is no display (the machine sleeps): what there is
/// stays as it is.
pub fn choose(
    profiles: &mut Vec<Profile>,
    displays: &[Display],
    workspace: Option<&str>,
    count: u64,
) -> Option<Chosen> {
    if displays.is_empty() {
        return None;
    }
    let key = ProfileKey::of(displays, workspace);
    let (index, made) = match profiles.iter().position(|profile| profile.key == key) {
        Some(index) => (index, None),
        None => {
            let from = nearest(profiles, &key);
            let mut profile = from.map(|index| profiles[index].clone()).unwrap_or_default();
            let from = from.map(|index| profiles[index].key.clone());
            profile.key = key;
            profiles.push(profile);
            while profiles.len() > MOST {
                let oldest = (0..profiles.len() - 1)
                    .min_by_key(|index| profiles[*index].last_used)
                    .unwrap_or(0);
                profiles.remove(oldest);
            }
            (profiles.len() - 1, Some(from))
        }
    };
    let profile = &mut profiles[index];
    profile.last_used = count;
    // (Also for one that was there: a file changed by hand may name a
    // display its own key does not have.)
    let moved = rehome(profile, displays);
    Some(Chosen { index, made, moved })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Anchor;
    use gevrek_model::geometry::Rect;

    fn display(id: &str, x: i32, width: i32, height: i32) -> Display {
        Display::new(id, Rect::new(x, 0, width, height))
    }

    fn geometry(display: &str, anchor: Anchor) -> ZoneGeometry {
        ZoneGeometry {
            display: DisplayId::new(display),
            anchor,
            offset_x: 20,
            offset_y: 30,
            width: 300,
            height: 200,
        }
    }

    fn laptop() -> Display {
        Display { primary: true, ..display("laptop", 0, 1920, 1080) }
    }

    #[test]
    fn the_same_displays_in_any_order_are_the_same_set() {
        let one = [laptop(), display("desk", 1920, 2560, 1440)];
        let other = [display("desk", -2560, 2560, 1440), laptop()];
        assert_eq!(ProfileKey::of(&one, None), ProfileKey::of(&other, None));
        // Another size, another scale, another workspace: another set.
        let larger = [laptop(), display("desk", 1920, 3840, 2160)];
        assert_ne!(ProfileKey::of(&one, None), ProfileKey::of(&larger, None));
        let scaled = [laptop(), Display { scale: 1.5, ..display("desk", 1920, 2560, 1440) }];
        assert_ne!(ProfileKey::of(&one, None), ProfileKey::of(&scaled, None));
        assert_ne!(ProfileKey::of(&one, None), ProfileKey::of(&one, Some("2")));
    }

    #[test]
    fn the_first_arrangement_is_made_from_nothing() {
        let mut profiles = Vec::new();
        let chosen = choose(&mut profiles, &[laptop()], None, 1).unwrap();
        assert_eq!(chosen, Chosen { index: 0, made: Some(None), moved: vec![] });
        assert_eq!(profiles.len(), 1);
        // The same displays again: the same one, nothing made.
        let again = choose(&mut profiles, &[laptop()], None, 2).unwrap();
        assert_eq!(again, Chosen { index: 0, made: None, moved: vec![] });
        assert_eq!(profiles[0].last_used, 2);
        // No display: nothing is chosen and nothing changes.
        assert_eq!(choose(&mut profiles, &[], None, 3), None);
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn zones_of_a_display_that_went_move_over_and_are_back_when_it_is() {
        let both = [laptop(), display("desk", 1920, 2560, 1440)];
        let mut profiles = Vec::new();
        choose(&mut profiles, &both, None, 1).unwrap();
        profiles[0].geometry.insert(ZoneId::new("left"), geometry("laptop", Anchor::TopLeft));
        profiles[0].geometry.insert(ZoneId::new("right"), geometry("desk", Anchor::BottomRight));
        let docked = profiles[0].clone();

        let alone = choose(&mut profiles, &[laptop()], None, 2).unwrap();
        assert_eq!(alone.made, Some(Some(docked.key.clone())));
        assert_eq!(alone.moved, [ZoneId::new("right")]);
        let moved = &profiles[alone.index].geometry[&ZoneId::new("right")];
        // The same corner, the same distance, on the display that is left.
        assert_eq!(moved, &geometry("laptop", Anchor::BottomRight));
        assert_eq!(moved.on(&laptop()), Rect::new(1600, 850, 300, 200));

        let back = choose(&mut profiles, &both, None, 3).unwrap();
        assert_eq!((back.index, &back.made, back.moved.len()), (0, &None, 0));
        assert_eq!(profiles[0].geometry, docked.geometry);
    }

    #[test]
    fn a_new_set_takes_after_the_nearest_one() {
        let desk = display("desk", 1920, 2560, 1440);
        let beamer = display("beamer", 1920, 1280, 720);
        let mut profiles = Vec::new();
        let mark = |profiles: &mut Vec<Profile>, displays: &[Display], count: u64, zone: &str| {
            let chosen = choose(profiles, displays, None, count).unwrap();
            let on = displays.last().unwrap().id.as_str().to_string();
            profiles[chosen.index]
                .geometry
                .insert(ZoneId::new(zone), geometry(&on, Anchor::TopLeft));
            profiles[chosen.index].key.clone()
        };
        let alone = mark(&mut profiles, &[laptop()], 1, "from-alone");
        let docked = mark(&mut profiles, &[laptop(), desk.clone()], 2, "from-docked");
        let _presenting = mark(&mut profiles, &[laptop(), beamer.clone()], 3, "from-presenting");

        // The desk display at another resolution: nearest is the one with
        // the desk display, though the one with the beamer was used last.
        let sharper = Display { rect: Rect::new(1920, 0, 3840, 2160), ..desk.clone() };
        let chosen = choose(&mut profiles, &[laptop(), sharper], None, 4).unwrap();
        assert_eq!(chosen.made, Some(Some(docked.clone())));
        assert!(chosen.moved.is_empty());

        // All three: two in common with either pair; the pairs have
        // nothing that is missing, so the one used last wins.
        let chosen = choose(&mut profiles, &[laptop(), desk.clone(), beamer], None, 5).unwrap();
        assert_ne!(chosen.made, Some(Some(alone.clone())));
        assert_eq!(chosen.made.clone().unwrap().unwrap().displays.len(), 2);

        // A display never seen, alone: the laptop's own arrangement is
        // nearer than any with a display that is not there... none shares
        // a display, none is missing nothing, so the last used is taken,
        // and its zones go to the display there is.
        let stranger = display("stranger", 0, 1024, 768);
        let chosen = choose(&mut profiles, &[stranger], None, 6).unwrap();
        assert!(chosen.made.is_some());
        assert!(!chosen.moved.is_empty());
        let profile = &profiles[chosen.index];
        assert!(profile.geometry.values().all(|geometry| geometry.display.as_str() == "stranger"));
    }

    #[test]
    fn the_arrangement_used_longest_ago_is_forgotten_when_there_are_too_many() {
        let mut profiles = Vec::new();
        let sized = |width: i32| [display("laptop", 0, width, 1080)];
        for count in 0..MOST as i32 {
            choose(&mut profiles, &sized(1000 + count), None, count as u64 + 1).unwrap();
        }
        assert_eq!(profiles.len(), MOST);
        // The first one is used again, so the second is the oldest now.
        let first = choose(&mut profiles, &sized(1000), None, 500).unwrap();
        assert_eq!((first.index, first.made), (0, None));
        let new = choose(&mut profiles, &sized(5000), None, 501).unwrap();
        assert_eq!(profiles.len(), MOST);
        assert_eq!(new.index, MOST - 1);
        assert_eq!(profiles[new.index].key, ProfileKey::of(&sized(5000), None));
        let kept = |width: i32| profiles.iter().any(|profile| profile.key == ProfileKey::of(&sized(width), None));
        assert!(kept(1000) && !kept(1001) && kept(1002));
    }

    #[test]
    fn a_workspace_without_an_arrangement_takes_after_the_same_displays() {
        let mut profiles = Vec::new();
        let first = choose(&mut profiles, &[laptop()], Some("1"), 1).unwrap();
        profiles[first.index].geometry.insert(ZoneId::new("z"), geometry("laptop", Anchor::Top));
        profiles[first.index].hidden.insert(ZoneId::new("hidden"));
        let second = choose(&mut profiles, &[laptop()], Some("2"), 2).unwrap();
        assert_ne!(second.index, first.index);
        assert_eq!(profiles[second.index].geometry, profiles[first.index].geometry);
        assert_eq!(profiles[second.index].hidden, profiles[first.index].hidden);
        // Each is its own from then on.
        profiles[second.index].geometry.clear();
        assert!(!profiles[first.index].geometry.is_empty());
    }
}
