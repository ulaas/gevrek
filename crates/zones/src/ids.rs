//! The names things are known by. Each is a string that stays the same
//! over restarts: a zone's from the configuration, an item's from the
//! backend (a shell parsing name, a file URI), a display's from the
//! backend as well (derived from what the display says of itself, never
//! its place in a list).

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, schemars::JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(text: &str) -> Self {
                Self(text.to_string())
            }
        }
    };
}

/// A zone, as the configuration names it.
pub use gevrek_model::zones::ZoneId;

id!(
    /// An item on the desktop or in a folder a zone shows.
    ItemId
);
id!(
    /// A display. The same display has the same id on whichever port and
    /// in whichever order it is found.
    DisplayId
);
