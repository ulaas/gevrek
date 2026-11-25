//! The zones as a screen reader is told of them: for each zone and each
//! item its role, its name, what state it is in and which one it is
//! among how many. Made from a scene and nothing else, so what is read
//! out is what is drawn; whoever draws turns this into the system's own
//! tree (AccessKit) without having to know the zones.
//!
//! What a covered zone holds is not in the scene, and so is not here: a
//! covered zone says that it is covered, and how many items it holds.

use crate::core::{Scene, ZoneScene};
use crate::layout::PlacedItem;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The whole desktop: the root.
    Desktop,
    /// A zone: a group with a name that holds items.
    Zone,
    /// The items of a display that are in no zone.
    Loose,
    /// One item.
    Item,
}

/// One node of the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessNode {
    /// The same for the same thing from one scene to the next.
    pub id: String,
    pub role: Role,
    /// What it is called.
    pub name: String,
    /// What else is worth saying of it, in words: what a zone shows, how
    /// many items, that it is locked or covered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What is typed into it (a zone's filter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// On the desktop, in logical units, without any scrolling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    /// Nothing can be dragged into or out of it, or it cannot be moved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// For what can be rolled up: whether its items are to be seen now.
    /// None for what cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// What it holds is covered from view.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
    /// It is not drawn now (put away, or a tab that is not in front).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Which one it is among its like, counted from 1; 0 where that
    /// means nothing.
    #[serde(default)]
    pub position: u32,
    /// How many its like are.
    #[serde(default)]
    pub of: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AccessNode>,
}

impl AccessNode {
    fn new(id: String, role: Role, name: String) -> Self {
        Self {
            id,
            role,
            name,
            description: String::new(),
            value: None,
            rect: None,
            focused: false,
            selected: false,
            locked: false,
            expanded: None,
            masked: false,
            hidden: false,
            position: 0,
            of: 0,
            children: Vec::new(),
        }
    }

    /// This node and everything below it, the node itself first.
    pub fn walk(&self) -> Vec<&AccessNode> {
        let mut nodes = vec![self];
        for child in &self.children {
            nodes.extend(child.walk());
        }
        nodes
    }
}

fn counted(count: usize) -> String {
    match count {
        0 => "no items".to_string(),
        1 => "1 item".to_string(),
        count => format!("{count} items"),
    }
}

fn item(placed: &PlacedItem, within: &str, origin: (i32, i32), hidden: bool) -> AccessNode {
    let mut node = AccessNode::new(
        format!("{within}/item:{}", placed.item),
        Role::Item,
        placed.name.clone(),
    );
    node.rect = Some(Rect::new(
        origin.0 + placed.rect.x,
        origin.1 + placed.rect.y,
        placed.rect.width,
        placed.rect.height,
    ));
    node.focused = placed.focused;
    node.selected = placed.selected;
    node.position = placed.position;
    node.of = placed.of;
    node.hidden = hidden;
    node
}

/// What is said of a zone beside its name.
fn describe(zone: &ZoneScene) -> String {
    let mut parts = Vec::new();
    if let Some(portal) = &zone.portal {
        let way: Vec<&str> = portal
            .crumbs
            .iter()
            .map(|crumb| crumb.name.as_str())
            .collect();
        parts.push(format!("folder {}", way.join(", ")));
        match &portal.listing {
            crate::portal::Listing::Loading => parts.push("loading".to_string()),
            crate::portal::Listing::Unavailable { .. } => parts.push("not available".to_string()),
            crate::portal::Listing::Ready => {}
        }
    }
    if let Some(tab) = &zone.tab {
        parts.push(format!("tab {} of {}", tab.index + 1, tab.count));
    }
    if zone.masked {
        parts.push("covered".to_string());
    }
    match &zone.filter {
        Some(_) if !zone.masked => parts.push(format!(
            "{} of {} shown",
            zone.items.items.len(),
            counted(zone.item_count)
        )),
        _ => parts.push(counted(zone.item_count)),
    }
    if zone.lock.geometry || zone.lock.members {
        parts.push("locked".to_string());
    }
    parts.join(", ")
}

/// The scene as a tree: the desktop, in it the zones in the order of the
/// scene and the loose items of each display, in them the items.
pub fn tree(scene: &Scene) -> AccessNode {
    let mut root = AccessNode::new("desktop".to_string(), Role::Desktop, "Desktop".to_string());
    let reachable: Vec<&ZoneScene> = scene
        .zones
        .iter()
        .filter(|zone| zone.shown && zone.front)
        .collect();
    for zone in &scene.zones {
        let id = format!("zone:{}", zone.zone);
        let name = if zone.title.trim().is_empty() {
            "Untitled zone".to_string()
        } else {
            zone.title.clone()
        };
        let mut node = AccessNode::new(id.clone(), Role::Zone, name);
        node.description = describe(zone);
        node.value = zone.filter.clone();
        node.rect = Some(zone.rect);
        node.focused = zone.focused;
        node.locked = zone.lock.geometry || zone.lock.members;
        node.expanded = zone.rolled_up.then_some(zone.expanded);
        node.masked = zone.masked;
        node.hidden = !zone.shown || !zone.front;
        if let Some(at) = reachable.iter().position(|shown| shown.zone == zone.zone) {
            node.position = at as u32 + 1;
            node.of = reachable.len() as u32;
        }
        // Of a zone that is rolled up only the title is there: its items
        // are not to be walked to until it is open.
        let closed = zone.rolled_up && !zone.expanded;
        node.children = zone
            .items
            .items
            .iter()
            .map(|placed| {
                item(
                    placed,
                    &id,
                    (zone.content.x, zone.content.y),
                    node.hidden || closed,
                )
            })
            .collect();
        root.children.push(node);
    }
    for loose in &scene.loose {
        let id = format!("loose:{}", loose.display);
        let mut node = AccessNode::new(id.clone(), Role::Loose, "Desktop items".to_string());
        node.description = if loose.masked {
            format!("covered, {}", counted(loose.item_count))
        } else {
            counted(loose.item_count)
        };
        node.rect = Some(loose.area);
        node.masked = loose.masked;
        node.focused = loose.focused;
        // (Put away with the one key, but for what was said to stay.)
        let away = scene.loose_hidden;
        node.children = loose
            .items
            .iter()
            .map(|placed| {
                item(
                    placed,
                    &id,
                    (loose.area.x, loose.area.y),
                    away && !placed.always_visible,
                )
            })
            .collect();
        root.children.push(node);
    }
    root
}
