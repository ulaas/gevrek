//! What gevrek's display manager and its greeter say to each other (the
//! service itself is the program `gevrek-dm`).

pub mod wire;

/// Where the greeter finds the service's socket (in its environment).
pub const SOCKET_VARIABLE: &str = "GEVREK_DM_SOCKET";

/// The user the greeter runs as (made by the package: sysusers).
pub const GREETER_USER: &str = "gevrek-greeter";
