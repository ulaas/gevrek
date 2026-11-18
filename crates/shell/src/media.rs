//! The keys for what is playing: play and pause, next, previous, stop.
//!
//! Players announce themselves on the session's bus (the MPRIS
//! specification); a key is passed on to the one that is playing, or to
//! the first one there is when none is.

use zbus::blocking::Connection;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What a key asks of a player (the names of the specification's calls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    PlayPause,
    Next,
    Previous,
    Stop,
}

impl Ask {
    fn call(self) -> &'static str {
        match self {
            Ask::PlayPause => "PlayPause",
            Ask::Next => "Next",
            Ask::Previous => "Previous",
            Ask::Stop => "Stop",
        }
    }
}

/// What a player has on: the piece's title, who it is by, and whether it
/// is playing now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Playing {
    /// The player, by the name it has on the bus (`firefox`, `spotify`).
    pub player: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// The piece's picture, where the player names a file of it.
    pub art: Option<crate::control::Art>,
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

/// The player a key goes to, of those on the bus: the one that is
/// playing, else the first there is.
fn player_on(connection: &Connection) -> Option<(String, bool)> {
    let names: Vec<String> = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let mut players: Vec<String> = names
        .into_iter()
        .filter(|name| name.starts_with(PREFIX))
        .collect();
    players.sort();
    let is_playing = |name: &String| {
        property(connection, name, "PlaybackStatus")
            .and_then(|value| String::try_from(value).ok())
            .is_some_and(|status| status == "Playing")
    };
    match players.iter().find(|name| is_playing(name)) {
        Some(name) => Some((name.clone(), true)),
        None => players.into_iter().next().map(|name| (name, false)),
    }
}

/// A property of a player.
fn property(
    connection: &Connection,
    player: &str,
    name: &str,
) -> Option<zbus::zvariant::OwnedValue> {
    connection
        .call_method(
            Some(player),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(PLAYER, name),
        )
        .ok()?
        .body()
        .deserialize::<zbus::zvariant::OwnedValue>()
        .ok()
}

/// What is on: of the player that is playing, else of the first there
/// is. `None` when there is no player (or no bus).
pub fn playing() -> Option<Playing> {
    let connection = Connection::session().ok()?;
    let (player, playing) = player_on(&connection)?;
    let facts = property(&connection, &player, "Metadata")
        .and_then(|value| {
            std::collections::HashMap::<String, zbus::zvariant::OwnedValue>::try_from(value).ok()
        })
        .unwrap_or_default();
    let title = facts
        .get("xesam:title")
        .and_then(|value| <&str>::try_from(value).ok())
        .unwrap_or_default()
        .to_string();
    // (Those it is by come as a list.)
    let artist = facts
        .get("xesam:artist")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .map(|names| names.join(", "))
        .unwrap_or_default();
    // (The piece's picture, where it is a file on this machine: one on
    // the net is not fetched.)
    let art = facts
        .get("mpris:artUrl")
        .and_then(|value| <&str>::try_from(value).ok())
        .and_then(file_of)
        .and_then(|file| std::fs::read(file).ok())
        .and_then(|bytes| crate::control::Art::of_file(&bytes));
    Some(Playing {
        player: player.trim_start_matches(PREFIX).to_string(),
        title,
        artist,
        playing,
        art,
    })
}

/// The file an address of the `file://` kind names (`%20` and the like
/// read as what they stand for); nothing of any other address.
fn file_of(address: &str) -> Option<std::path::PathBuf> {
    let path = address.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let pair = [rest.next()?, rest.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&pair).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    path.starts_with('/').then(|| path.into())
}

/// Pass a key on to a player. Returns the name of the player that was
/// told, or `None` when there is none (or no bus).
pub fn tell(ask: Ask) -> Option<String> {
    let connection = Connection::session().ok()?;
    let (player, _) = player_on(&connection)?;
    connection
        .call_method(Some(player.as_str()), PATH, Some(PLAYER), ask.call(), &())
        .ok()?;
    Some(player.trim_start_matches(PREFIX).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_on_is_said_in_one_line() {
        let on = Playing {
            player: "firefox".to_string(),
            title: "A Song".to_string(),
            artist: "Somebody".to_string(),
            playing: true,
            art: None,
        };
        assert_eq!(on.line(), "A Song  ·  Somebody");
        // The piece's picture is a file named by its address; one on the
        // net is none.
        assert_eq!(
            file_of("file:///home/ada/My%20Music/cover.png"),
            Some("/home/ada/My Music/cover.png".into())
        );
        for no_file in ["https://i.scdn.co/image/ab67", "file://relative", "file:///a%2", ""] {
            assert_eq!(file_of(no_file), None, "{no_file}");
        }
        assert_eq!(
            Playing {
                artist: String::new(),
                ..on.clone()
            }
            .line(),
            "A Song"
        );
        // Nothing said of the piece: the player.
        assert_eq!(
            Playing {
                title: " ".to_string(),
                ..on
            }
            .line(),
            "firefox"
        );
    }

    struct Player;

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        #[zbus(property)]
        fn playback_status(&self) -> String {
            "Playing".to_string()
        }

        #[zbus(property)]
        fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
            use zbus::zvariant::Value;
            let own = |value: Value<'_>| value.try_to_owned().expect("a plain value");
            [
                ("xesam:title".to_string(), own(Value::from("A Song"))),
                (
                    "xesam:artist".to_string(),
                    own(Value::from(vec!["Somebody", "Somebody Else"])),
                ),
            ]
            .into_iter()
            .collect()
        }

        fn next(&self) {}
    }

    /// Against a bus of its own, never the session's:
    /// `dbus-run-session -- cargo test -p gevrek-shell on_a_bus -- --ignored`
    #[test]
    #[ignore = "needs a session bus of its own (dbus-run-session)"]
    fn a_player_on_a_bus_says_what_it_has_on() {
        assert_eq!(playing(), None, "no player yet");
        let _player = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.mpris.MediaPlayer2.gevrek_check"))
            .and_then(|builder| builder.serve_at(PATH, Player))
            .and_then(|builder| builder.build())
            .expect("a name on the bus");
        let on = playing().expect("a player");
        assert_eq!(on.player, "gevrek_check");
        assert_eq!(on.title, "A Song");
        assert_eq!(on.artist, "Somebody, Somebody Else");
        assert!(on.playing);
        assert_eq!(tell(Ask::Next).as_deref(), Some("gevrek_check"));
    }
}
