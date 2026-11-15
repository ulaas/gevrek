//! A recording: a film of a display, of a part of it, or of one window,
//! written as a WebM file beside the screenshots.
//!
//! The pictures come from the compositor as they change (the way it
//! gives them to any recorder: `screencast::Grab`), on a thread of the
//! recording's own, and are handed to GStreamer's command
//! (`gst-launch-1.0`, with the VP8 encoder that every installation of it
//! has), which writes the file. The film has a fixed number of pictures
//! a second: where nothing changed, the last one is shown again, so the
//! film is as long as the recording was. A window is recorded by its own
//! pictures, wherever it is moved to; made larger or smaller meanwhile,
//! it is shown in the room the film began with. With sound, what the
//! machine plays is in the film too (the sound server's monitor of what
//! is played, encoded as Opus). Without GStreamer's command there is no
//! recording, and that is said.

use crate::screencast::{Grab, What};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Pictures a second.
pub const RATE: u32 = 15;
/// What turns the pictures into a file.
const ENCODER: &str = "gst-launch-1.0";
/// How long the first picture is waited for.
const PATIENCE: Duration = Duration::from_secs(5);

/// How many pictures a film of this length has had by now.
pub fn pictures_due(elapsed: Duration, rate: u32) -> u64 {
    (elapsed.as_secs_f64() * f64::from(rate)).floor() as u64 + 1
}

/// What the encoder is asked: pictures of this size, blue first, from
/// its standard input, into a WebM file; with `sound`, what the machine
/// plays beside them.
pub fn pipeline(width: u32, height: u32, rate: u32, file: &Path, sound: bool) -> Vec<String> {
    let mut words: Vec<String> = [
        "-q",
        "-e",
        "fdsrc",
        "fd=0",
        "!",
        "rawvideoparse",
        "format=bgrx",
    ]
    .iter()
    .map(|word| word.to_string())
    .collect();
    words.push(format!("width={width}"));
    words.push(format!("height={height}"));
    words.push(format!("framerate={rate}/1"));
    for word in [
        "!",
        "videoconvert",
        "!",
        "vp8enc",
        "deadline=1",
        "cpu-used=8",
        "!",
        "queue",
        "!",
        "film.",
    ] {
        words.push(word.to_string());
    }
    if sound {
        // What is played, as the sound server lets it be listened to.
        for word in [
            "pulsesrc",
            "device=@DEFAULT_MONITOR@",
            "!",
            "audioconvert",
            "!",
            "audioresample",
            "!",
            "opusenc",
            "!",
            "queue",
            "!",
            "film.",
        ] {
            words.push(word.to_string());
        }
    }
    for word in ["webmmux", "name=film", "!", "filesink"] {
        words.push(word.to_string());
    }
    words.push(format!("location={}", file.display()));
    words
}

/// Whether what the machine plays can be recorded: GStreamer has the
/// piece that listens to the sound server.
pub fn sound_can_be_recorded() -> bool {
    Command::new("gst-inspect-1.0")
        .arg("pulsesrc")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The file a recording begun now is written to, in `folder`.
pub fn file_in(folder: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let mut path = folder.join(format!("Recording {stamp}.webm"));
    let mut number = 2;
    while path.exists() {
        path = folder.join(format!("Recording {stamp} ({number}).webm"));
        number += 1;
    }
    path
}

/// A part of a display, in its own pixels: where it begins and how
/// large it is.
pub type Part = (u32, u32, u32, u32);

/// The size a film of a picture of this size has (of `part` of it,
/// where one is named): cut to what the picture has, and to even sides
/// (the encoder takes no odd ones). `None`: nothing is left.
pub fn film_size(width: u32, height: u32, part: Option<Part>) -> Option<(u32, u32)> {
    let (wide, high) = match part {
        Some((x, y, part_width, part_height)) => (
            part_width.min(width.saturating_sub(x)),
            part_height.min(height.saturating_sub(y)),
        ),
        None => (width, height),
    };
    let (wide, high) = (wide & !1, high & !1);
    (wide > 0 && high > 0).then_some((wide, high))
}

/// Put a picture (or `part` of it) into a film's picture of a set size:
/// what of it lies in that room, from the corner; black where the
/// picture is smaller than the room (a window that was made smaller).
/// `out` is made as large as the film's picture is.
pub fn fit(
    (width, height): (u32, u32),
    pixels: &[u8],
    part: Option<Part>,
    (film_width, film_height): (u32, u32),
    out: &mut Vec<u8>,
) {
    out.clear();
    out.resize((film_width * film_height * 4) as usize, 0);
    let (from_x, from_y) = part.map_or((0, 0), |(x, y, ..)| (x, y));
    if from_x >= width || from_y >= height || pixels.len() < (width * height * 4) as usize {
        return;
    }
    let wide = film_width.min(width - from_x) as usize;
    let high = film_height.min(height - from_y) as usize;
    for row in 0..high {
        let read = ((from_y as usize + row) * width as usize + from_x as usize) * 4;
        let write = row * film_width as usize * 4;
        out[write..write + wide * 4].copy_from_slice(&pixels[read..read + wide * 4]);
    }
}

/// What is recorded.
#[derive(Debug, Clone, PartialEq)]
pub enum Of {
    /// A display (the one named, else the first), or a part of it.
    Display {
        output: Option<String>,
        part: Option<Part>,
    },
    /// The window with this title, of this program.
    Window { title: String, app_id: String },
}

/// The thing the compositor is asked pictures of, for what is recorded.
fn what_of(of: &Of) -> anyhow::Result<What> {
    let all = crate::screencast::listed(false)?;
    let found = match of {
        Of::Display { output, .. } => all.into_iter().find(|source| match (&source.what, output) {
            (What::Output(name), Some(wanted)) => name == wanted,
            (What::Output(_), None) => true,
            _ => false,
        }),
        Of::Window { title, app_id } => all.into_iter().find(|source| {
            matches!(source.what, What::Window(_))
                && source.label == *title
                && source.detail == *app_id
        }),
    };
    found
        .map(|source| source.what)
        .ok_or_else(|| anyhow::anyhow!("what is to be recorded is not there"))
}

/// A recording that is going on.
pub struct Recording {
    stop: Arc<AtomicBool>,
    pub file: PathBuf,
    /// Whether what the machine plays is in it.
    pub sound: bool,
    thread: std::thread::JoinHandle<Result<u64, String>>,
}

/// Begin recording into `folder`; with `sound`, what the machine plays
/// too (left out, and `Recording::sound` says so, where GStreamer cannot
/// listen to it). An error when the encoder is not installed or no
/// picture of what is to be recorded can be taken.
pub fn start(of: Of, folder: &Path, sound: bool) -> anyhow::Result<Recording> {
    std::fs::create_dir_all(folder)?;
    let file = file_in(folder);
    let sound = sound && sound_can_be_recorded();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let written_to = file.clone();
    // The thread says when the film has begun, or why there is none.
    let (begun, began) = mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new()
        .name("gevrek-record".to_string())
        .spawn(move || -> Result<u64, String> {
            let fail = |why: String| {
                let _ = begun.send(Err(why.clone()));
                Err(why)
            };
            let part = match &of {
                Of::Display { part, .. } => *part,
                Of::Window { .. } => None,
            };
            let mut grab = match what_of(&of).and_then(|what| Grab::open(&what, true)) {
                Ok(grab) => grab,
                Err(error) => return fail(format!("{error:#}")),
            };
            // The first picture says how large the film is, and whether
            // there are pictures at all.
            let mut last = Vec::new();
            let size = match grab.next(PATIENCE) {
                Ok(Some((width, height))) => {
                    let Some(size) = film_size(width, height, part) else {
                        return fail("what is to be recorded is too small".to_string());
                    };
                    fit((width, height), grab.pixels(), part, size, &mut last);
                    size
                }
                Ok(None) => return fail("the compositor gave no picture".to_string()),
                Err(error) => return fail(format!("{error:#}")),
            };
            let mut encoder = match Command::new(ENCODER)
                .args(pipeline(size.0, size.1, RATE, &written_to, sound))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(encoder) => encoder,
                Err(error) => {
                    return fail(format!(
                        "{ENCODER} is needed to record (GStreamer's tools): {error}"
                    ))
                }
            };
            let Some(mut pictures) = encoder.stdin.take() else {
                return fail("the encoder takes no pictures".to_string());
            };
            let _ = begun.send(Ok(()));
            let started = Instant::now();
            let mut written = 0u64;
            let mut failed = None;
            loop {
                // As many pictures as the film is to have by now: the
                // last one again where nothing changed.
                let due = pictures_due(started.elapsed(), RATE);
                while written < due {
                    if let Err(error) = pictures.write_all(&last) {
                        failed = Some(format!("the encoder stopped: {error}"));
                        break;
                    }
                    written += 1;
                }
                if failed.is_some() || stopped.load(Ordering::Relaxed) {
                    break;
                }
                // Until the next picture is due: a new one, if one comes.
                let next = Duration::from_secs_f64(written as f64 / f64::from(RATE));
                let wait = next
                    .checked_sub(started.elapsed())
                    .unwrap_or(Duration::ZERO)
                    .min(Duration::from_millis(200));
                let before = Instant::now();
                match grab.next(wait) {
                    Ok(Some((width, height))) => {
                        fit((width, height), grab.pixels(), part, size, &mut last);
                        // (One that came early: the rest of the time.)
                        if let Some(rest) = wait.checked_sub(before.elapsed()) {
                            std::thread::sleep(rest);
                        }
                    }
                    Ok(None) => {}
                    // What was recorded is gone: the film ends.
                    Err(error) => {
                        log::debug!("the recording ends: {error:#}");
                        break;
                    }
                }
            }
            // The end of the pictures is the end of the film: the encoder
            // is told to finish the file (the sound has no end of its
            // own to wait for).
            drop(pictures);
            if sound {
                // SAFETY: the encoder's own process, which is still this
                // thread's child: asked to end as Ctrl+C asks it.
                unsafe {
                    libc::kill(encoder.id() as libc::pid_t, libc::SIGINT);
                }
            }
            let ended = encoder.wait().map_err(|error| error.to_string())?;
            match failed {
                Some(why) => Err(why),
                None if !ended.success() => Err(format!("the encoder ended with {ended}")),
                None => Ok(written),
            }
        })?;
    match began.recv_timeout(PATIENCE * 2) {
        Ok(Ok(())) => Ok(Recording {
            stop,
            file,
            sound,
            thread,
        }),
        Ok(Err(why)) => anyhow::bail!(why),
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            anyhow::bail!("the recording did not begin")
        }
    }
}

impl Recording {
    /// End the recording and wait for the file to be finished. The file
    /// and how many pictures it has.
    pub fn finish(self) -> Result<(PathBuf, u64), String> {
        self.stop.store(true, Ordering::Relaxed);
        let pictures = self
            .thread
            .join()
            .map_err(|_| "the recording broke off".to_string())??;
        Ok((self.file, pictures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_film_has_as_many_pictures_as_its_length_asks_for() {
        assert_eq!(pictures_due(Duration::ZERO, 15), 1);
        assert_eq!(pictures_due(Duration::from_millis(66), 15), 1);
        assert_eq!(pictures_due(Duration::from_millis(67), 15), 2);
        assert_eq!(pictures_due(Duration::from_secs(2), 15), 31);
    }

    #[test]
    fn the_encoder_is_told_the_size_the_rate_and_the_file() {
        let words = pipeline(1280, 720, 15, Path::new("/tmp/a film.webm"), false);
        assert!(words.contains(&"width=1280".to_string()));
        assert!(words.contains(&"height=720".to_string()));
        assert!(words.contains(&"framerate=15/1".to_string()));
        assert_eq!(
            words.last().map(String::as_str),
            Some("location=/tmp/a film.webm")
        );
        // The pictures go into the file's maker by its name; no sound.
        assert_eq!(words.iter().filter(|word| *word == "film.").count(), 1);
        assert!(!words.iter().any(|word| word == "pulsesrc"));
        // With sound: what is played, into the same file.
        let with = pipeline(1280, 720, 15, Path::new("/tmp/f.webm"), true);
        assert_eq!(with.iter().filter(|word| *word == "film.").count(), 2);
        assert!(with.contains(&"device=@DEFAULT_MONITOR@".to_string()));
        assert!(with.contains(&"opusenc".to_string()));
    }

    #[test]
    fn a_film_is_as_large_as_what_is_recorded_with_even_sides() {
        assert_eq!(film_size(1280, 720, None), Some((1280, 720)));
        assert_eq!(film_size(1281, 721, None), Some((1280, 720)));
        // A part: cut to what the picture has of it.
        assert_eq!(
            film_size(1280, 720, Some((100, 100, 301, 201))),
            Some((300, 200))
        );
        assert_eq!(
            film_size(1280, 720, Some((1200, 700, 400, 400))),
            Some((80, 20))
        );
        assert_eq!(film_size(1280, 720, Some((1280, 0, 10, 10))), None);
        assert_eq!(film_size(1, 5, None), None);
    }

    #[test]
    fn a_picture_is_put_into_the_films_room() {
        // Three by three, each pixel its own number.
        let pixels: Vec<u8> = (0..9u8)
            .flat_map(|pixel| [pixel, pixel, pixel, 255])
            .collect();
        let firsts = |out: &[u8]| out.chunks(4).map(|pixel| pixel[0]).collect::<Vec<u8>>();
        let mut out = Vec::new();
        // The whole into two by two: its upper left.
        fit((3, 3), &pixels, None, (2, 2), &mut out);
        assert_eq!(firsts(&out), [0, 1, 3, 4]);
        // A part from the middle.
        fit((3, 3), &pixels, Some((1, 1, 2, 2)), (2, 2), &mut out);
        assert_eq!(firsts(&out), [4, 5, 7, 8]);
        // A picture smaller than the room (the window was made smaller):
        // black beyond it.
        fit((3, 3), &pixels, None, (4, 4), &mut out);
        assert_eq!(out.len(), 64);
        assert_eq!(firsts(&out)[..4], [0, 1, 2, 0]);
        assert_eq!(firsts(&out)[12..], [0, 0, 0, 0]);
        // Fewer bytes than the size says: all black, no fault.
        fit((3, 3), &pixels[..8], None, (2, 2), &mut out);
        assert_eq!(firsts(&out), [0, 0, 0, 0]);
    }
}
