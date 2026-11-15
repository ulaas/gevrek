// Generated protobuf code
pub mod proto {
    // Every call answers with `tonic::Status` when it fails, which is a
    // large value; the generated code is not ours to change.
    #![allow(clippy::result_large_err)]
    tonic::include_proto!("gevrek");
}

// Re-export commonly used types
pub use proto::*;

// Utilities
pub mod client;
/// Model types to and from their wire form.
pub mod conversions;

/// Where the window manager listens and is reached.
pub mod endpoint;
pub use endpoint::Endpoint;

/// The address every gevrek program uses unless told otherwise: the local
/// endpoint of this instance (`endpoint.rs`).
pub const DEFAULT_WM_SOCKET: &str = endpoint::LOCAL;
pub const DEFAULT_WM_ENDPOINT: &str = endpoint::LOCAL;

/// The TCP address the window manager used before it had a local endpoint.
/// Still what `--address 127.0.0.1:57651` means; nothing listens there
/// unless asked to.
pub const LEGACY_TCP_ADDRESS: &str = "127.0.0.1:57651";

// Re-export client and retry config
pub use client::{RetryConfig, WMClient};
