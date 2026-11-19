//! Whether a folder a portal shows can be reached, in the words both
//! systems use.
//!
//! A portal may show a folder on another machine, on a drive that is
//! pulled out, or one that somebody deletes while it is shown. None of
//! that may hold up whoever draws: a folder is opened, read and watched
//! on a thread of its own, the caller is answered at once, and what is in
//! the folder is told when it is known (`BackendEvent::Listed`). A folder
//! that does not answer within [`PATIENCE`] is told as away
//! (`BackendEvent::ListingFailed`); when it answers after all, or comes
//! back, it is told whole.

use gevrek_zones::backend::{BackendError, BackendEvent, Location};
use std::time::Duration;

/// How long a folder may take to answer before it is told as away. (It
/// is still waited for after that, by its own thread: a machine that is
/// slow rather than gone is shown when it answers.)
pub const PATIENCE: Duration = Duration::from_secs(4);

/// How a watched folder stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// It is being opened and read; nothing was told of it yet.
    Opening,
    /// It was read and is watched.
    There,
    /// It is not there, or does not answer: for a message to the person.
    Away { why: String },
}

impl Reach {
    pub fn is_away(&self) -> bool {
        matches!(self, Reach::Away { .. })
    }
}

/// What the core is told when a folder went away: that it could not be
/// listed, and why. (Its coming back needs no word of its own: a listing
/// makes it good again.)
pub fn told(location: &Location, reach: &Reach) -> Vec<BackendEvent> {
    match reach {
        Reach::Away { why } => vec![BackendEvent::ListingFailed {
            location: location.clone(),
            error: BackendError::NotFound { what: why.clone() },
        }],
        Reach::Opening | Reach::There => Vec::new(),
    }
}
