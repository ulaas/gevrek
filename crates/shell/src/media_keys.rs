//! What is playing, on Windows.
//!
//! On Linux the shell talks to the players on the session's bus
//! (`media.rs`). On Windows a player tells the system what it has on, and
//! the system is asked (`gevrek_system::playing`): the piece, who it is
//! by, whether it plays, its picture. A player that tells the system
//! nothing is not known here; the keyboard's own keys for it (play and
//! pause, next, previous, stop) reach it all the same, and are what is
//! pressed where the system knows of no player.

pub use crate::control::Art;

/// What a player is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    PlayPause,
    Next,
    Previous,
    #[allow(dead_code)]
    Stop,
}

/// What a player has on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Playing {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// The piece's picture, where the player gave one.
    pub art: Option<Art>,
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

    /// Whether this is the same piece on the same player as another.
    fn same_piece(&self, other: &Playing) -> bool {
        self.player == other.player && self.title == other.title && self.artist == other.artist
    }
}

/// What is playing, as the system says it now. `before`: what it said
/// last; the picture of a piece that is the same is kept, not read and
/// made small again.
pub fn playing(before: Option<&Playing>) -> Option<Playing> {
    let kept = |title: &str, artist: &str| {
        before.is_some_and(|before| {
            before.art.is_some() && before.title == title && before.artist == artist
        })
    };
    let now = gevrek_system::playing::playing(|title, artist| !kept(title, artist))?;
    let mut playing = Playing {
        player: now.player,
        title: now.title,
        artist: now.artist,
        playing: now.playing,
        art: now.picture.as_deref().and_then(Art::of_file),
    };
    if playing.art.is_none() {
        playing.art = before
            .filter(|before| before.same_piece(&playing))
            .and_then(|before| before.art.clone());
    }
    Some(playing)
}

/// Ask whatever plays: the player the system knows of, and where it
/// knows of none (or the player did not take it), by the keyboard's key
/// for it. (Beside the shell: the system is waited for.)
pub fn tell(ask: Ask) {
    std::thread::spawn(move || {
        let asked = match ask {
            Ask::PlayPause => gevrek_system::playing::Ask::PlayPause,
            Ask::Next => gevrek_system::playing::Ask::Next,
            Ask::Previous => gevrek_system::playing::Ask::Previous,
            Ask::Stop => gevrek_system::playing::Ask::Stop,
        };
        if gevrek_system::playing::tell(asked) {
            log::debug!("the player the system knows is told {ask:?}");
            return;
        }
        log::debug!("no player took {ask:?}: the keyboard's own key for it is pressed");
        gevrek_system::every_key::press(match ask {
            Ask::PlayPause => 0xB3,
            Ask::Next => 0xB0,
            Ask::Previous => 0xB1,
            Ask::Stop => 0xB2,
        });
    });
}
