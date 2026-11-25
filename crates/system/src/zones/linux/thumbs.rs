//! Small pictures of what files hold, by the Thumbnail Managing Standard.
//!
//! Every program of a desktop shares one store, `$XDG_CACHE_HOME/thumbnails`:
//!
//! - `normal/` (128 pixels at most on a side), `large/` (256), `x-large/`
//!   (512), `xx-large/` (1024). A thumbnail is a PNG named by the MD5 of
//!   the address of its file, and carries in its text chunks whose it is
//!   (`Thumb::URI`) and when that file was last changed (`Thumb::MTime`).
//!   Only when both still fit is it the file's picture.
//! - `fail/<program>/`: an empty picture of the same name says that this
//!   program tried and could not, so it does not try again until the file
//!   changes.
//!
//! What is here reads the store, and fills it for the pictures gevrek can
//! read itself (PNG, JPEG, WebP), so that a file manager finds them too.
//! Thumbnails of everything else (films, documents) are made by the
//! desktop's thumbnailers, and are used when they are there.
//!
//! A thumbnail is written as the standard asks: into a temporary file in
//! the same folder, readable by the person alone (0600, the folders
//! 0700), then moved into place, so no reader ever sees half of one.
//!
//! PNG's text chunks are read and written by hand: the picture library
//! does neither.

use super::places::Places;
use crate::icons::Picture;
use std::path::{Path, PathBuf};

/// The folders of the store with the side their pictures have at most,
/// the smallest first.
pub const SIZES: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

/// A file larger than this is not read to make a picture of it.
const LARGEST_FILE: u64 = 64 * 1024 * 1024;
/// Nor a picture with more pixels than this (it would be held unpacked).
const MOST_PIXELS: u64 = 80_000_000;

/// The first eight bytes of every PNG.
const PNG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The store's folder.
pub fn store(places: &Places) -> PathBuf {
    places.cache_home.join("thumbnails")
}

/// The name a thumbnail's file has: the MD5 of the address, and `.png`.
pub fn name_of(uri: &str) -> String {
    format!("{}.png", super::md5::hex(uri.as_bytes()))
}

/// Where the thumbnail of an address is in one of the folders.
pub fn path_in(places: &Places, folder: &str, uri: &str) -> PathBuf {
    store(places).join(folder).join(name_of(uri))
}

/// Where gevrek notes that it could not make a picture of a file.
fn failure_of(places: &Places, uri: &str) -> PathBuf {
    store(places)
        .join("fail")
        .join(format!("gevrek-{}", env!("CARGO_PKG_VERSION")))
        .join(name_of(uri))
}

/// The folders to look in for a picture of `pixels` on a side: the
/// smallest that is large enough first, then the larger ones (a larger
/// picture made smaller is as good), then the smaller ones, the largest
/// first (better than none). The second answer is how many of them are
/// large enough.
fn folders_for(pixels: u32) -> (Vec<(&'static str, u32)>, usize) {
    let first = SIZES
        .iter()
        .position(|(_, side)| *side >= pixels)
        .unwrap_or(SIZES.len() - 1);
    let mut order: Vec<(&str, u32)> = SIZES[first..].to_vec();
    order.extend(SIZES[..first].iter().rev());
    (order, SIZES.len() - first)
}

/// The CRC every chunk of a PNG ends with (ISO 3309, as in zlib).
fn crc(parts: &[&[u8]]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in parts.iter().copied().flatten() {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The text chunks (`tEXt`) of a PNG: keyword and value. They come
/// before the picture's own data in a thumbnail, so reading stops there;
/// a file that is no PNG has none.
pub fn texts_of(png: &[u8]) -> Vec<(String, String)> {
    let mut texts = Vec::new();
    let Some(mut rest) = png.strip_prefix(PNG) else {
        return texts;
    };
    while rest.len() >= 12 {
        let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let kind = &rest[4..8];
        let Some(data) = rest.get(8..8 + length) else {
            break;
        };
        if kind == b"tEXt" {
            if let Some(end) = data.iter().position(|&byte| byte == 0) {
                // (Latin-1 by the PNG standard: each byte is one letter.)
                let latin =
                    |bytes: &[u8]| bytes.iter().map(|&byte| byte as char).collect::<String>();
                texts.push((latin(&data[..end]), latin(&data[end + 1..])));
            }
        } else if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        match rest.get(12 + length..) {
            Some(after) => rest = after,
            None => break,
        }
    }
    texts
}

/// A PNG with text chunks put in right after its header chunk.
fn with_texts(png: &[u8], texts: &[(&str, String)]) -> Option<Vec<u8>> {
    // The signature, then the header chunk: 4 of length, 4 of name, 13 of
    // data, 4 of CRC.
    const HEADER_END: usize = 8 + 4 + 4 + 13 + 4;
    if !png.starts_with(PNG) || png.get(12..16) != Some(b"IHDR") || png.len() < HEADER_END {
        return None;
    }
    let mut out = png[..HEADER_END].to_vec();
    for (key, value) in texts {
        let mut data = key.as_bytes().to_vec();
        data.push(0);
        data.extend(value.bytes());
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(b"tEXt");
        out.extend(&data);
        out.extend(crc(&[b"tEXt", &data]).to_be_bytes());
    }
    out.extend(&png[HEADER_END..]);
    Some(out)
}

/// Whether the thumbnail at `path` is the picture of the file with this
/// address as it is now: it names the address, and the time the file was
/// changed at is the one it was made for. (Where it also says how large
/// the file was, that has to fit as well.)
pub fn is_current(path: &Path, uri: &str, modified: u64, size: u64) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    // The text chunks are at the start; a thumbnailer that put megabytes
    // before them did not write a thumbnail.
    let mut start = Vec::new();
    if std::io::Read::read_to_end(&mut std::io::Read::take(file, 64 * 1024), &mut start).is_err() {
        return false;
    }
    let texts = texts_of(&start);
    let said = |key: &str| {
        texts
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    said("Thumb::URI") == Some(uri)
        && said("Thumb::MTime").and_then(|time| time.parse::<u64>().ok()) == Some(modified)
        && said("Thumb::Size").is_none_or(|said| said.parse::<u64>().ok() == Some(size))
}

/// The current thumbnail of a file in the store, if one is there, for a
/// picture of about `pixels` on a side: its file.
pub fn find(places: &Places, uri: &str, modified: u64, size: u64, pixels: u32) -> Option<PathBuf> {
    find_among(places, uri, modified, size, &folders_for(pixels).0)
}

fn find_among(
    places: &Places,
    uri: &str,
    modified: u64,
    size: u64,
    folders: &[(&'static str, u32)],
) -> Option<PathBuf> {
    folders
        .iter()
        .map(|(folder, _)| path_in(places, folder, uri))
        .find(|path| is_current(path, uri, modified, size))
}

/// Whether gevrek can read a file of this type as a picture itself.
pub fn can_make(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/webp")
}

/// What is known of the file a thumbnail is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source<'a> {
    pub path: &'a Path,
    pub uri: &'a str,
    pub mime: &'a str,
    /// Seconds since 1970.
    pub modified: u64,
    pub size: u64,
}

/// The small picture of a file at about `pixels` on a side: from the
/// store where a current one is there, else made and put into the store.
/// `None` where there is none and none can be made. It reads and writes
/// files: not for the thread that draws.
pub fn picture(places: &Places, source: &Source<'_>, pixels: u32) -> Option<Picture> {
    let pixels = pixels.clamp(8, 1024);
    // The store's own pictures get none: they would beget each other.
    if source.path.starts_with(store(places)) {
        return crate::icons::load(source.path, pixels);
    }
    let (folders, enough) = folders_for(pixels);
    let kept = |folders: &[(&'static str, u32)]| {
        find_among(places, source.uri, source.modified, source.size, folders)
            .and_then(|found| crate::icons::load(&found, pixels))
    };
    // One that is large enough is as good as a new one.
    if let Some(picture) = kept(&folders[..enough]) {
        return Some(picture);
    }
    let failure = failure_of(places, source.uri);
    if !can_make(source.mime) || is_current(&failure, source.uri, source.modified, source.size) {
        // None can be made here: a smaller one, shown larger, is better
        // than none.
        return kept(&folders[enough..]);
    }
    let (folder, side) = folders[0];
    match make(source, side) {
        Some(small) => {
            let png = encode(&small, source)?;
            let path = path_in(places, folder, source.uri);
            if let Err(error) = put(&path, &png) {
                // It can still be shown; only the store goes without.
                log::debug!("thumbnail of {} not kept: {error}", source.uri);
            }
            let shown = small
                .resize(pixels, pixels, image::imageops::FilterType::Triangle)
                .to_rgba8();
            Some(Picture {
                width: shown.width(),
                height: shown.height(),
                pixels: shown.into_raw(),
            })
        }
        None => {
            // Written down, so that a picture that cannot be read is not
            // tried again each time the folder is shown.
            let empty = image::DynamicImage::new_rgba8(1, 1);
            if let Some(png) = encode(&empty, source) {
                if let Err(error) = put(&failure, &png) {
                    log::debug!(
                        "zones: that no thumbnail could be made of {} is not kept: {error}",
                        source.uri
                    );
                }
            }
            kept(&folders[enough..])
        }
    }
}

/// Read a file as a picture no larger than `side` on its longer side. A
/// picture that is smaller stays as it is: a thumbnail is never blown up.
fn make(source: &Source<'_>, side: u32) -> Option<image::DynamicImage> {
    if source.size > LARGEST_FILE {
        return None;
    }
    // (Read without the file counting as used: see `open_quietly`.)
    let open = || {
        let file = std::io::BufReader::new(super::open_quietly(source.path).ok()?);
        image::ImageReader::new(file).with_guessed_format().ok()
    };
    let (width, height) = open()?.into_dimensions().ok()?;
    if u64::from(width) * u64::from(height) > MOST_PIXELS {
        return None;
    }
    let full = open()?.decode().ok()?;
    Some(if width > side || height > side {
        full.resize(side, side, image::imageops::FilterType::Triangle)
    } else {
        full
    })
}

/// A thumbnail as the bytes of its file: the picture as PNG, with the
/// text chunks the standard asks for.
fn encode(small: &image::DynamicImage, source: &Source<'_>) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    small
        .to_rgba8()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    with_texts(
        &png,
        &[
            ("Thumb::URI", source.uri.to_string()),
            ("Thumb::MTime", source.modified.to_string()),
            ("Thumb::Size", source.size.to_string()),
            ("Thumb::Mimetype", source.mime.to_string()),
            ("Software", format!("gevrek {}", env!("CARGO_PKG_VERSION"))),
        ],
    )
}

/// Put a thumbnail's bytes into the store at `path`, whole or not at all.
fn put(path: &Path, png: &[u8]) -> std::io::Result<()> {
    let folder = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    super::private_folder(folder)?;
    super::write_private(path, png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn picture_file(path: &Path, width: u32, height: u32) {
        image::save_buffer(
            path,
            &[200u8, 30, 30, 255].repeat((width * height) as usize),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("saved");
    }

    fn source<'a>(path: &'a Path, uri: &'a str, mime: &'a str) -> Source<'a> {
        let data = std::fs::metadata(path).expect("there");
        Source {
            path,
            uri,
            mime,
            modified: super::super::seconds(data.modified().ok()).unwrap_or(0),
            size: data.len(),
        }
    }

    #[test]
    fn a_thumbnail_is_named_by_the_sum_of_its_address() {
        let places = Places::under(Path::new("/x"));
        // The example of the standard.
        assert_eq!(
            path_in(&places, "normal", "file:///home/jens/photos/me.png"),
            PathBuf::from("/x/home/.cache/thumbnails/normal/c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
        let folders = |pixels| {
            folders_for(pixels)
                .0
                .into_iter()
                .map(|(folder, _)| folder)
                .collect::<Vec<_>>()
        };
        assert_eq!(folders(48), ["normal", "large", "x-large", "xx-large"]);
        assert_eq!(folders(200), ["large", "x-large", "xx-large", "normal"]);
        assert_eq!(folders(4000), ["xx-large", "x-large", "large", "normal"]);
        // The check value of the CRC every PNG reader knows.
        assert_eq!(crc(&[b"1234", b"56789"]), 0xcbf4_3926);
    }

    #[test]
    fn a_thumbnail_is_made_kept_and_found_again() {
        let root = crate::zones::linux::testing::folder("thumbs");
        let places = Places::under(&root);
        let file = root.join("wide picture.png");
        picture_file(&file, 400, 100);
        let uri = super::super::uris::uri_of(&file);
        let of = source(&file, &uri, "image/png");
        assert_eq!(find(&places, &uri, of.modified, of.size, 96), None);

        let shown = picture(&places, &of, 96).expect("a picture");
        // It keeps its shape, within what was asked for.
        assert_eq!((shown.width, shown.height), (96, 24));
        assert_eq!(&shown.pixels[..4], &[200, 30, 30, 255]);

        // It is in the store, in the folder for that size, for the person
        // alone, and says whose it is.
        let kept = path_in(&places, "normal", &uri);
        assert_eq!(
            find(&places, &uri, of.modified, of.size, 96),
            Some(kept.clone())
        );
        let rights =
            |path: &Path| std::fs::metadata(path).expect("there").permissions().mode() & 0o777;
        assert_eq!(rights(&kept), 0o600);
        assert_eq!(rights(kept.parent().expect("a folder")), 0o700);
        assert_eq!(rights(&store(&places)), 0o700);
        let bytes = std::fs::read(&kept).expect("read");
        let texts = texts_of(&bytes);
        let said = |key: &str| {
            texts
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(said("Thumb::URI"), Some(uri.clone()));
        assert_eq!(said("Thumb::MTime"), Some(of.modified.to_string()));
        assert_eq!(said("Thumb::Size"), Some(of.size.to_string()));
        assert_eq!(said("Thumb::Mimetype").as_deref(), Some("image/png"));
        // What was written is a picture any reader reads, of the store's size.
        let read = image::open(&kept).expect("a PNG");
        assert_eq!((read.width(), read.height()), (128, 32));
        // Nothing is left of the writing.
        let left: Vec<_> = std::fs::read_dir(kept.parent().expect("a folder"))
            .expect("listed")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left.len(), 1);

        // The second time it comes from the store: the file itself may be
        // unreadable by now.
        let again = picture(
            &places,
            &Source {
                mime: "application/x-unknown",
                ..of.clone()
            },
            96,
        )
        .expect("kept");
        assert_eq!((again.width, again.height), (96, 24));

        // The file changed: the kept one is not its picture any more.
        assert!(!is_current(&kept, &uri, of.modified + 1, of.size));
        assert!(!is_current(&kept, &uri, of.modified, of.size + 1));
        assert!(!is_current(&kept, "file:///another", of.modified, of.size));
        assert_eq!(find(&places, &uri, of.modified + 1, of.size, 96), None);

        // A film's thumbnail that is too small is still shown, larger.
        let blown = picture(
            &places,
            &Source {
                mime: "video/x-matroska",
                ..of.clone()
            },
            200,
        )
        .expect("the small one");
        assert_eq!((blown.width, blown.height), (200, 50));
        // A larger size goes into its own folder; a picture smaller than
        // that folder's side is kept as large as it is.
        let large = picture(&places, &of, 300).expect("a picture");
        assert_eq!((large.width, large.height), (300, 75));
        let kept_large = path_in(&places, "x-large", &uri);
        let read = image::open(&kept_large).expect("a PNG");
        assert_eq!((read.width(), read.height()), (400, 100));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn another_programs_thumbnail_is_used_and_checked() {
        let root = crate::zones::linux::testing::folder("thumbs-other");
        let places = Places::under(&root);
        // A film gevrek cannot read, with a thumbnail a thumbnailer made.
        let file = root.join("film.mkv");
        std::fs::write(&file, b"not a picture").expect("written");
        let uri = super::super::uris::uri_of(&file);
        let of = source(&file, &uri, "video/x-matroska");
        assert!(picture(&places, &of, 64).is_none());
        let small = root.join("small.png");
        picture_file(&small, 256, 256);
        let plain = std::fs::read(&small).expect("read");
        let theirs = with_texts(
            &plain,
            &[
                ("Thumb::URI", uri.clone()),
                ("Thumb::MTime", of.modified.to_string()),
            ],
        )
        .expect("a PNG");
        let kept = path_in(&places, "large", &uri);
        std::fs::create_dir_all(kept.parent().expect("a folder")).expect("folders");
        std::fs::write(&kept, &theirs).expect("written");
        // Asked for small, the larger one is found and made smaller.
        let shown = picture(&places, &of, 64).expect("theirs");
        assert_eq!((shown.width, shown.height), (64, 64));
        // One that names another file, or no file, is not used.
        std::fs::write(&kept, &plain).expect("written");
        assert!(picture(&places, &of, 64).is_none());
        let stale = with_texts(
            &plain,
            &[
                ("Thumb::URI", uri.clone()),
                ("Thumb::MTime", "1".to_string()),
            ],
        )
        .expect("a PNG");
        std::fs::write(&kept, stale).expect("written");
        assert!(picture(&places, &of, 64).is_none());
        assert!(texts_of(b"no png at all").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_picture_that_cannot_be_read_is_tried_once() {
        let root = crate::zones::linux::testing::folder("thumbs-fail");
        let places = Places::under(&root);
        let file = root.join("broken.png");
        std::fs::write(&file, b"\x89PNG\r\n\x1a\nbut then nothing").expect("written");
        let uri = super::super::uris::uri_of(&file);
        let of = source(&file, &uri, "image/png");
        assert!(picture(&places, &of, 64).is_none());
        let failure = failure_of(&places, &uri);
        assert!(is_current(&failure, &uri, of.modified, of.size));
        assert!(failure.starts_with(store(&places).join("fail")));
        // It is a good file now, but as long as it has not changed it is
        // not tried again; once it has, it is.
        picture_file(&file, 10, 10);
        assert!(picture(
            &places,
            &Source {
                size: of.size,
                ..of.clone()
            },
            64
        )
        .is_none());
        let changed = Source {
            modified: of.modified + 5,
            size: 99,
            ..of.clone()
        };
        assert!(picture(&places, &changed, 64).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
