//! What the person sets: the zones there are, how they look, and the rules
//! that sort items into them. The types are the configuration's own
//! (`gevrek_model::zones`, the section `desktop.zones`); they are named
//! here as well, so that whoever uses the zones needs one crate.

pub use gevrek_model::zones::{
    AutoCategory, AutoZones, DateField, DeskGesture, DeskGestures, ExpandOn, Lock, Matcher,
    PrivacySettings, RollUpSettings, Rule, RuleAction, SmartSource, SnapSettings, Sort, TimedRules,
    TitleBar, View, WhenShared, ZoneDefinition, ZoneKind, ZoneLook, ZonesConfig, DEFAULT_ICON_SIZE,
};
