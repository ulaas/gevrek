//! Identities of windows and displays, and of engine objects.
//!
//! Engine and model code only compare and store these; they never turn one
//! back into an OS handle (docs/PHASE2_PLAN.md, decision D2). Stage 1: the
//! Windows backend uses the HWND / HMONITOR value as the id and is the only
//! code that converts back (core's `HwndExt` / `HmonitorExt`). Stage 2
//! (Phase 2 step 6): it allocates fresh ids, so a reused HWND becomes a
//! different window.

use serde::{Deserialize, Serialize};

/// A top-level window.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct WindowId(pub isize);

/// A display output (stage 1: the HMONITOR value; stage 2: stable across
/// hotplug, allocated per device path).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct OutputId(pub isize);

macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
        pub struct $name(u64);

        impl $name {
            /// Wrap a value allocated by the engine.
            pub const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            /// The allocated value, for lookup tables and IPC.
            pub const fn raw(self) -> u64 {
                self.0
            }
        }
    };
}

opaque_id!(
    /// A workspace, unique for the lifetime of the WM.
    WorkspaceId
);
opaque_id!(
    /// A tiling container.
    ContainerId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_as_plain_numbers() {
        // Same JSON as the handle types they replace (persisted state).
        assert_eq!(serde_json::to_string(&WindowId(0x1234)).unwrap(), "4660");
        assert_eq!(
            serde_json::from_str::<OutputId>("65537").unwrap(),
            OutputId(65537)
        );
        let workspace = WorkspaceId::from_raw(7);
        assert_eq!(workspace.raw(), 7);
    }
}
