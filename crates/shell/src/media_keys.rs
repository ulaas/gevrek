//! What is playing, on Windows: the keys for it.
//!
//! On Linux the shell talks to the players on the session's bus
//! (`media.rs`) and so knows what is playing. Windows has no such bus the
//! shell reads; what every player there listens to is the keyboard's own
//! keys for it (play and pause, next, previous, stop), and those are what
//! is pressed here. So the control centre's buttons work with whatever
//! plays, and it does not say what that is.

/// What a player is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    PlayPause,
    Next,
    Previous,
    #[allow(dead_code)]
    Stop,
}

/// What is playing (never known here: see above).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Playing {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

impl Playing {
    /// One line of it: the title and who it is by, else the player.
    pub fn line(&self) -> String {
        match (self.title.trim(), self.artist.trim()) {
            ("", _) => self.player.clone(),
            (title, "") => title.to_string(),
            (title, artist) => format!("{title}  ·  {artist}"),
        }
    }
}

/// Ask whatever plays: by the keyboard's key for it.
pub fn tell(ask: Ask) {
    gevrek_system::every_key::press(match ask {
        Ask::PlayPause => 0xB3,
        Ask::Next => 0xB0,
        Ask::Previous => 0xB1,
        Ask::Stop => 0xB2,
    });
}
