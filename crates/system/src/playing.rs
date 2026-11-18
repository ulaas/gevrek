//! What is playing, as Windows knows it.
//!
//! A player on Windows tells the system what it has on (the same word
//! the system's own sound flyout and the lock screen show): its title,
//! who it is by, whether it plays now, and a picture. The system keeps
//! one such session for each player; they are asked here, and told to
//! play, to pause and to go on to the next.
//!
//! On Linux the shell asks the players themselves, on the session's bus
//! (`gevrek-shell`'s `media.rs`).

/// What a player has on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Playing {
    /// The player, by a name a person knows it by (`Spotify`).
    pub player: String,
    pub title: String,
    pub artist: String,
    /// Whether it plays now (not: is paused, or has stopped).
    pub playing: bool,
    /// The piece's picture, as the file the player gave (a PNG or a
    /// JPEG); nothing where it gave none.
    pub picture: Option<Vec<u8>>,
}

/// What a player is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    PlayPause,
    Next,
    Previous,
    Stop,
}

/// A player's name from what the system knows it by: the program's file
/// (`Spotify.exe`), or the name of a packaged application
/// (`SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`, `MSEdge`).
pub fn player_name(known_as: &str) -> String {
    let known_as = known_as.trim();
    // (A packaged application: what stands after the `!` is its own
    // name within the package, where that is one; else the package's.)
    let name = match known_as.rsplit_once('!') {
        Some((package, own)) if own.eq_ignore_ascii_case("app") || own.is_empty() => {
            let package = package.split('_').next().unwrap_or(package);
            package.rsplit('.').next().unwrap_or(package)
        }
        Some((_, own)) => own,
        None => known_as,
    };
    // (A file: without its folders and its ending.)
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let name = match name.rsplit_once('.') {
        Some((before, ending)) if ending.eq_ignore_ascii_case("exe") => before,
        _ => name,
    };
    // (A name with its maker's before it, `com.squirrel.TIDAL.TIDAL`:
    // its last part.)
    let name = name.rsplit('.').next().unwrap_or(name);
    let name = match name.to_ascii_lowercase().as_str() {
        "msedge" => "Edge",
        "chrome" => "Chrome",
        "firefox" => "Firefox",
        "zunemusic" => "Media Player",
        "zunevideo" => "Films & TV",
        "spotifymusic" => "Spotify",
        _ => name,
    };
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// What is playing: the session that plays now, else the one the system
/// has in front. Nothing where no player told the system anything.
/// `picture`: whether the piece's picture is read too (it is a file's
/// worth of bytes; one who has it already for this piece leaves it).
#[cfg(windows)]
pub fn playing(picture: impl Fn(&str, &str) -> bool) -> Option<Playing> {
    system::playing(picture)
}

/// Ask the player that `playing` tells of. Whether it took that.
#[cfg(windows)]
pub fn tell(ask: Ask) -> bool {
    system::tell(ask)
}

#[cfg(windows)]
mod system {
    use super::{Ask, Playing};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

    /// The largest picture that is read.
    const LARGEST: u64 = 8 * 1024 * 1024;

    fn plays(session: &Session) -> bool {
        session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .is_ok_and(|status| status == Status::Playing)
    }

    /// The session that is told of: one that plays, else the system's own
    /// choice.
    fn session() -> Option<Session> {
        let manager = Manager::RequestAsync().ok()?.get().ok()?;
        let playing = manager
            .GetSessions()
            .ok()
            .and_then(|sessions| sessions.into_iter().find(plays));
        playing.or_else(|| manager.GetCurrentSession().ok())
    }

    fn picture_of(from: &IRandomAccessStreamReference) -> Option<Vec<u8>> {
        let stream = from.OpenReadAsync().ok()?.get().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > LARGEST {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        let read = reader.LoadAsync(size as u32).ok()?.get().ok()?;
        let mut bytes = vec![0u8; read as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        Some(bytes)
    }

    pub fn playing(picture: impl Fn(&str, &str) -> bool) -> Option<Playing> {
        let session = session()?;
        let player = session
            .SourceAppUserModelId()
            .map(|name| super::player_name(&name.to_string()))
            .unwrap_or_default();
        let properties = session
            .TryGetMediaPropertiesAsync()
            .ok()
            .and_then(|asked| asked.get().ok());
        let said = |text: windows::core::Result<windows::core::HSTRING>| {
            text.map(|text| text.to_string()).unwrap_or_default()
        };
        let (title, artist) = properties
            .as_ref()
            .map(|properties| (said(properties.Title()), said(properties.Artist())))
            .unwrap_or_default();
        let picture = properties
            .as_ref()
            .filter(|_| picture(&title, &artist))
            .and_then(|properties| properties.Thumbnail().ok())
            .and_then(|thumbnail| picture_of(&thumbnail));
        Some(Playing {
            player,
            title,
            artist,
            playing: plays(&session),
            picture,
        })
    }

    pub fn tell(ask: Ask) -> bool {
        let Some(session) = session() else {
            return false;
        };
        let asked = match ask {
            Ask::PlayPause => session.TryTogglePlayPauseAsync(),
            Ask::Next => session.TrySkipNextAsync(),
            Ask::Previous => session.TrySkipPreviousAsync(),
            Ask::Stop => session.TryStopAsync(),
        };
        asked.and_then(|asked| asked.get()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_is_named_as_a_person_knows_it() {
        for (known_as, name) in [
            ("Spotify.exe", "Spotify"),
            ("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Spotify"),
            ("MSEdge", "Edge"),
            ("msedge.exe", "Edge"),
            ("chrome.exe", "Chrome"),
            ("Chrome", "Chrome"),
            ("firefox.exe", "Firefox"),
            ("308046B0AF4A39CB", "308046B0AF4A39CB"),
            ("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic", "Media Player"),
            ("Microsoft.ZuneMusic_8wekyb3d8bbwe!App", "Media Player"),
            (r"C:\Program Files\VideoLAN\VLC\vlc.exe", "Vlc"),
            ("com.squirrel.TIDAL.TIDAL", "TIDAL"),
            ("foobar2000.exe", "Foobar2000"),
            ("", ""),
            ("!", ""),
        ] {
            assert_eq!(player_name(known_as), name, "{known_as}");
        }
    }

    /// Asked of the system as it is: whatever plays on this machine, or
    /// nothing; the asking comes to its end.
    #[cfg(windows)]
    #[test]
    fn what_is_playing_is_asked_of_the_system() {
        let asked = playing(|_, _| true);
        if let Some(playing) = asked {
            assert!(playing.picture.is_none_or(|picture| !picture.is_empty()));
        }
    }
}
