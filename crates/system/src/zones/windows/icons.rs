//! Icons and small pictures of files, by handle.
//!
//! The core asks for a picture and is answered later with a handle; the
//! renderer gives the handle back and gets the pixels. Between the two is
//! the cache the panel and the launcher already use
//! (`crate::icons::Cache`): the shell is asked on the cache's own thread,
//! never on the one that draws, and each picture is asked for once.
//!
//! A picture is asked for in pixels (the logical size times the display's
//! scale), so it is as sharp as the display: the shell draws an icon at
//! any size from the largest it has.

use crate::icons::{Cache, Picture};
use gevrek_zones::backend::{BackendEvent, IconHandle, IconRequest, IconSize};
use gevrek_zones::ids::ItemId;
use std::collections::HashMap;
use std::sync::Arc;

/// What a picture is known by to the cache: `i` or `t` (icon, thumbnail),
/// how often the item changed since the start, and the item's id. The
/// count makes a picture asked for after a change another one than the
/// picture from before it, even while that one is still being made.
fn key(thumbnail: bool, change: u32, id: &str) -> String {
    format!("{}{change}|{id}", if thumbnail { 't' } else { 'i' })
}

/// A key taken apart again: whether it is a thumbnail, and the id.
fn parts(key: &str) -> Option<(bool, &str)> {
    let (head, id) = key.split_once('|')?;
    match head.chars().next()? {
        'i' => Some((false, id)),
        't' => Some((true, id)),
        _ => None,
    }
}

/// The side of a picture in pixels: the logical size at the display's
/// scale, within what the shell draws.
pub fn pixels(size: IconSize) -> u32 {
    let scale = if size.scale.is_finite() && size.scale > 0.0 { size.scale } else { 1.0 };
    ((f64::from(size.size) * scale).round() as u32).clamp(8, 512)
}

pub struct Icons {
    cache: Cache,
    wake: Arc<dyn Fn() + Send + Sync>,
    handles: HashMap<(String, u32), IconHandle>,
    keys: HashMap<IconHandle, (String, u32)>,
    /// How often each item changed.
    changes: HashMap<String, u32>,
    /// Asked for and not answered yet.
    waiting: Vec<(IconRequest, IconHandle)>,
    numbers: u64,
}

impl Icons {
    /// `wake` is called when a picture was made: the moment to call
    /// `drain` again.
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::with(look, wake)
    }

    /// With something else than the shell to make the pictures (a test).
    pub fn with(
        look: impl Fn(&str, u32) -> Option<Picture> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let woken = Arc::clone(&wake);
        Self {
            cache: Cache::with(look, move || woken()),
            wake,
            handles: HashMap::new(),
            keys: HashMap::new(),
            changes: HashMap::new(),
            waiting: Vec::new(),
            numbers: 0,
        }
    }

    /// Ask for an item's icon, or for the small picture of what it holds.
    pub fn request(&mut self, item: &ItemId, size: IconSize, thumbnail: bool) -> IconRequest {
        self.numbers += 1;
        let request = IconRequest(self.numbers);
        let change = self.changes.get(item.as_str()).copied().unwrap_or(0);
        let wanted = (key(thumbnail, change, item.as_str()), pixels(size));
        let handle = match self.handles.get(&wanted) {
            Some(handle) => *handle,
            None => {
                self.numbers += 1;
                let handle = IconHandle(self.numbers);
                self.handles.insert(wanted.clone(), handle);
                self.keys.insert(handle, wanted.clone());
                handle
            }
        };
        self.waiting.push((request, handle));
        // A picture that is there already wakes nobody by itself.
        if self.cache.answer(&wanted.0, wanted.1).is_some() {
            (self.wake)();
        }
        request
    }

    /// The answers that are there by now.
    pub fn drain(&mut self) -> Vec<BackendEvent> {
        let mut ready = Vec::new();
        let waiting = std::mem::take(&mut self.waiting);
        for (request, handle) in waiting {
            let answer = match self.keys.get(&handle) {
                Some((key, size)) => self.cache.answer(key, *size).map(|found| found.is_some()),
                // (Forgotten while it was asked for: the item changed or
                // went. Who asked asks again when told so.)
                None => Some(false),
            };
            match answer {
                Some(found) => ready.push(BackendEvent::IconReady {
                    request,
                    icon: found.then_some(handle),
                }),
                None => self.waiting.push((request, handle)),
            }
        }
        ready
    }

    /// The pixels of a picture that was answered with this handle: four
    /// bytes each (red, green, blue, how solid), not multiplied.
    pub fn picture(&mut self, handle: IconHandle) -> Option<&Picture> {
        let (key, size) = self.keys.get(&handle)?;
        self.cache.get(key, *size)
    }

    /// An item changed or went: its pictures are let go of. Asked for
    /// again, they are made again and have other handles.
    pub fn forget(&mut self, item: &ItemId) {
        let change = self.changes.entry(item.as_str().to_string()).or_insert(0);
        for thumbnail in [false, true] {
            self.cache.forget(&key(thumbnail, *change, item.as_str()));
        }
        *change += 1;
        let id = item.as_str();
        self.handles.retain(|(key, _), _| parts(key).is_none_or(|(_, of)| of != id));
        self.keys.retain(|_, (key, _)| parts(key).is_none_or(|(_, of)| of != id));
    }
}

/// Ask the shell for a picture by its key.
#[cfg(windows)]
fn look(key: &str, size: u32) -> Option<Picture> {
    let (thumbnail, id) = parts(key)?;
    crate::icons::shell::picture_as(&super::shell::plain(id), size, thumbnail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn size(size: u32, scale: f64) -> IconSize {
        IconSize { size, scale }
    }

    #[test]
    fn a_picture_is_asked_for_in_the_displays_pixels() {
        assert_eq!(pixels(size(48, 1.0)), 48);
        assert_eq!(pixels(size(48, 1.5)), 72);
        assert_eq!(pixels(size(48, 2.25)), 108);
        assert_eq!(pixels(size(400, 2.0)), 512);
        assert_eq!(pixels(size(2, 1.0)), 8);
        assert_eq!(pixels(size(32, f64::NAN)), 32);
        assert_eq!(pixels(size(32, 0.0)), 32);
    }

    #[test]
    fn a_key_says_what_it_is_a_picture_of() {
        assert_eq!(parts(&key(false, 0, "C:\\a|b.txt")), Some((false, "C:\\a|b.txt")));
        assert_eq!(parts(&key(true, 12, "::{X}")), Some((true, "::{X}")));
        assert_eq!(parts("nothing"), None);
    }

    #[test]
    fn pictures_come_as_handles_and_a_changed_item_gets_new_ones() {
        let (woken, wakes) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = woken.send(());
        });
        // Icons for everything; thumbnails only for what ends in png.
        let mut icons = Icons::with(
            |key, size| {
                let (thumbnail, id) = parts(key)?;
                (!thumbnail || id.ends_with(".png")).then(|| Picture {
                    width: size,
                    height: size,
                    pixels: vec![if thumbnail { 2 } else { 1 }; (size * size * 4) as usize],
                })
            },
            wake,
        );
        let wait = || wakes.recv_timeout(Duration::from_secs(10)).expect("woken");
        let (text, picture) = (ItemId::new("C:\\d\\a.txt"), ItemId::new("C:\\d\\a.png"));
        let icon = icons.request(&text, size(16, 2.0), false);
        let none = icons.request(&text, size(16, 2.0), true);
        let thumbnail = icons.request(&picture, size(16, 1.0), true);
        wait();
        wait();
        wait();
        let mut handles = HashMap::new();
        for event in icons.drain() {
            let BackendEvent::IconReady { request, icon } = event else {
                panic!("only pictures are answered here");
            };
            handles.insert(request, icon);
        }
        assert_eq!(handles.len(), 3);
        assert_eq!(handles[&none], None);
        let icon = handles[&icon].expect("an icon");
        let thumbnail = handles[&thumbnail].expect("a thumbnail");
        assert_ne!(icon, thumbnail);
        // The pixels are the display's: 16 at a scale of two is 32.
        assert_eq!(icons.picture(icon).map(|picture| (picture.width, picture.pixels[0])), Some((32, 1)));
        assert_eq!(icons.picture(thumbnail).map(|picture| (picture.width, picture.pixels[0])), Some((16, 2)));
        assert!(icons.drain().is_empty());

        // Asked for again, it is the same picture, answered at once.
        let again = icons.request(&text, size(16, 2.0), false);
        wait();
        assert_eq!(icons.drain(), [BackendEvent::IconReady { request: again, icon: Some(icon) }]);

        // The item changed: another handle, and the old one names nothing.
        icons.forget(&text);
        assert!(icons.picture(icon).is_none());
        let anew = icons.request(&text, size(16, 2.0), false);
        wait();
        let [BackendEvent::IconReady { request, icon: Some(new) }] = icons.drain()[..] else {
            panic!("one answer with a picture");
        };
        assert_eq!(request, anew);
        assert_ne!(new, icon);
        assert!(icons.picture(new).is_some());
        assert!(icons.picture(thumbnail).is_some());
    }
}
