//! Icons and small pictures of items, by handle, on both systems.
//!
//! The core asks for a picture and is answered later with a handle; the
//! renderer gives the handle back and gets the pixels. Between the two is
//! the cache the panel and the launcher already use
//! (`crate::icons::Cache`): a picture is made on the cache's own thread,
//! never on the one that draws, each once, and only when it is asked for
//! (nothing is made ahead for items nobody looks at).
//!
//! # How much is kept
//!
//! Pixels are large: a small picture of 256 pixels a side is a quarter of
//! a megabyte, and a folder a portal shows can hold thousands. So the
//! pictures that are kept are bounded in bytes ([`Pictures::set_budget`];
//! [`BUDGET`] to begin with). When more is held, pictures are let go of:
//! first those the renderer already took (it keeps what it drew as a
//! texture of its own, so that copy here is one nobody reads twice), the
//! one taken longest ago first; then, if that is not enough, those nobody
//! took yet, the oldest first. A handle whose picture was let go of names
//! nothing any more ([`Pictures::picture`] gives none), exactly as after
//! its item changed; asked for again, the picture is made again and has
//! another handle.
//!
//! What one `drain` answered is never let go of by that same `drain`:
//! whoever is told "it is there" finds it there. The bound can be passed
//! by that much for a moment.

use crate::icons::{Cache, Picture};
use gevrek_zones::backend::{BackendEvent, IconHandle, IconRequest};
use gevrek_zones::ids::ItemId;
use std::collections::HashMap;
use std::sync::Arc;

/// How many bytes of pixels are kept unless something else is said: room
/// for some three thousand icons of 64 pixels, or two hundred small
/// pictures of 256.
pub const BUDGET: usize = 48 * 1024 * 1024;

/// What a picture is known by to the cache: `i` or `t` (icon, small
/// picture), how often the item changed since the start, and the item's
/// id. The count makes a picture asked for after a change another one
/// than the picture from before it, even while that one is still being
/// made.
fn key(thumbnail: bool, change: u32, id: &str) -> String {
    format!("{}{change}|{id}", if thumbnail { 't' } else { 'i' })
}

/// A key taken apart again: whether it is a small picture, and the id.
fn parts(key: &str) -> Option<(bool, &str)> {
    let (head, id) = key.split_once('|')?;
    match head.chars().next()? {
        'i' => Some((false, id)),
        't' => Some((true, id)),
        _ => None,
    }
}

/// A picture that is held: how many bytes its pixels are, and when it was
/// last used (by the count of [`Pictures`], not by the clock).
struct Held {
    bytes: usize,
    used: u64,
    /// The renderer took its pixels at least once.
    taken: bool,
}

pub struct Pictures {
    cache: Cache,
    wake: Arc<dyn Fn() + Send + Sync>,
    handles: HashMap<(String, u32), IconHandle>,
    keys: HashMap<IconHandle, (String, u32)>,
    /// How often each item changed.
    changes: HashMap<String, u32>,
    /// Asked for and not answered yet.
    waiting: Vec<(IconRequest, IconHandle)>,
    numbers: u64,
    /// The pictures whose pixels are in the cache now.
    held: HashMap<IconHandle, Held>,
    /// The bytes of all of them together.
    bytes: usize,
    budget: usize,
    /// Counts every use, so that it can be said which was longest ago.
    uses: u64,
}

impl Pictures {
    /// `look` makes a picture: of the item with this id, this many pixels
    /// a side, its icon or (`true`) the small picture of what it holds.
    /// It runs on the cache's thread. `wake` is called when a picture was
    /// made: the moment to call `drain` again.
    pub fn with(
        look: impl Fn(&str, u32, bool) -> Option<Picture> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let woken = Arc::clone(&wake);
        let by_key = move |key: &str, pixels: u32| {
            let (thumbnail, id) = parts(key)?;
            look(id, pixels, thumbnail)
        };
        Self {
            cache: Cache::with(by_key, move || woken()),
            wake,
            handles: HashMap::new(),
            keys: HashMap::new(),
            changes: HashMap::new(),
            waiting: Vec::new(),
            numbers: 0,
            held: HashMap::new(),
            bytes: 0,
            budget: BUDGET,
            uses: 0,
        }
    }

    /// Keep at most this many bytes of pixels from now on.
    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.trim(self.uses + 1);
    }

    /// The bytes of the pixels that are kept now.
    pub fn held_bytes(&self) -> usize {
        self.bytes
    }

    /// Ask for an item's icon, or for the small picture of what it holds,
    /// `pixels` a side.
    pub fn request(&mut self, item: &ItemId, pixels: u32, thumbnail: bool) -> IconRequest {
        self.numbers += 1;
        let request = IconRequest(self.numbers);
        let change = self.changes.get(item.as_str()).copied().unwrap_or(0);
        let wanted = (key(thumbnail, change, item.as_str()), pixels);
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
        self.uses += 1;
        let now = self.uses;
        for (request, handle) in std::mem::take(&mut self.waiting) {
            let answer = match self.keys.get(&handle) {
                Some((key, size)) => self
                    .cache
                    .answer(key, *size)
                    .map(|found| found.map(|picture| picture.pixels.len())),
                // (Let go of while it was asked for: the item changed or
                // went. Who asked asks again when told so.)
                None => Some(None),
            };
            match answer {
                Some(found) => {
                    if let Some(bytes) = found {
                        self.note(handle, bytes, now);
                    }
                    ready.push(BackendEvent::IconReady {
                        request,
                        icon: found.map(|_| handle),
                    });
                }
                None => self.waiting.push((request, handle)),
            }
        }
        self.trim(now);
        ready
    }

    /// The pixels of a picture that was answered with this handle: four
    /// bytes each (red, green, blue, how solid), not multiplied. None
    /// when the item changed since, or the picture was let go of to keep
    /// within the bound: ask for it again.
    pub fn picture(&mut self, handle: IconHandle) -> Option<&Picture> {
        let (key, size) = self.keys.get(&handle)?;
        self.uses += 1;
        if let Some(held) = self.held.get_mut(&handle) {
            held.used = self.uses;
            held.taken = true;
        }
        self.cache.get(key, *size)
    }

    /// An item changed or went: its pictures are let go of. Asked for
    /// again, they are made again and have other handles.
    pub fn forget(&mut self, item: &ItemId) {
        let id = item.as_str();
        let change = self.changes.entry(id.to_string()).or_insert(0);
        for thumbnail in [false, true] {
            self.cache.forget(&key(thumbnail, *change, id));
        }
        *change += 1;
        let of_item = |key: &str| parts(key).is_some_and(|(_, of)| of == id);
        self.handles.retain(|(key, _), _| !of_item(key));
        let (held, bytes) = (&mut self.held, &mut self.bytes);
        self.keys.retain(|handle, (key, _)| {
            if !of_item(key) {
                return true;
            }
            if let Some(gone) = held.remove(handle) {
                *bytes -= gone.bytes;
            }
            false
        });
    }

    /// A picture is there: counted, and used now.
    fn note(&mut self, handle: IconHandle, bytes: usize, now: u64) {
        match self.held.get_mut(&handle) {
            Some(held) => held.used = now,
            None => {
                self.held.insert(
                    handle,
                    Held {
                        bytes,
                        used: now,
                        taken: false,
                    },
                );
                self.bytes += bytes;
            }
        }
    }

    /// Let go of pictures until what is kept fits the bound: those that
    /// were taken before those that were not, and of each the one used
    /// longest ago first. What was used at `now` stays whatever it weighs.
    fn trim(&mut self, now: u64) {
        while self.bytes > self.budget {
            let oldest = self
                .held
                .iter()
                .filter(|(_, held)| held.used < now)
                .min_by_key(|(handle, held)| (!held.taken, held.used, handle.0))
                .map(|(handle, _)| *handle);
            let Some(handle) = oldest else {
                break;
            };
            if let Some(gone) = self.held.remove(&handle) {
                self.bytes -= gone.bytes;
            }
            if let Some((key, size)) = self.keys.remove(&handle) {
                self.cache.forget_at(&key, size);
                self.handles.remove(&(key, size));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Pictures made by the test: icons for everything, small pictures
    /// only for what ends in png; each `pixels` a side.
    fn pictures() -> (Pictures, mpsc::Receiver<()>) {
        let (woken, wakes) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = woken.send(());
        });
        let pictures = Pictures::with(
            |id, size, thumbnail| {
                (!thumbnail || id.ends_with(".png")).then(|| Picture {
                    width: size,
                    height: size,
                    pixels: vec![if thumbnail { 2 } else { 1 }; (size * size * 4) as usize],
                })
            },
            wake,
        );
        (pictures, wakes)
    }

    /// Wait for the answers to `count` requests and give them by request.
    fn answers(
        pictures: &mut Pictures,
        wakes: &mpsc::Receiver<()>,
        count: usize,
    ) -> HashMap<IconRequest, Option<IconHandle>> {
        let mut handles = HashMap::new();
        while handles.len() < count {
            wakes.recv_timeout(Duration::from_secs(10)).expect("woken");
            for event in pictures.drain() {
                let BackendEvent::IconReady { request, icon } = event else {
                    panic!("only pictures are answered here");
                };
                handles.insert(request, icon);
            }
        }
        handles
    }

    #[test]
    fn a_key_says_what_it_is_a_picture_of() {
        assert_eq!(
            parts(&key(false, 0, "C:\\a|b.txt")),
            Some((false, "C:\\a|b.txt"))
        );
        assert_eq!(parts(&key(true, 12, "::{X}")), Some((true, "::{X}")));
        assert_eq!(parts("nothing"), None);
    }

    #[test]
    fn pictures_come_as_handles_and_a_changed_item_gets_new_ones() {
        let (mut icons, wakes) = pictures();
        let (text, picture) = (ItemId::new("C:\\d\\a.txt"), ItemId::new("C:\\d\\a.png"));
        let icon = icons.request(&text, 32, false);
        let none = icons.request(&text, 32, true);
        let thumbnail = icons.request(&picture, 16, true);
        let handles = answers(&mut icons, &wakes, 3);
        assert_eq!(handles[&none], None);
        let icon = handles[&icon].expect("an icon");
        let thumbnail = handles[&thumbnail].expect("a thumbnail");
        assert_ne!(icon, thumbnail);
        assert_eq!(
            icons
                .picture(icon)
                .map(|picture| (picture.width, picture.pixels[0])),
            Some((32, 1))
        );
        assert_eq!(
            icons
                .picture(thumbnail)
                .map(|picture| (picture.width, picture.pixels[0])),
            Some((16, 2))
        );
        assert!(icons.drain().is_empty());
        assert_eq!(icons.held_bytes(), 32 * 32 * 4 + 16 * 16 * 4);

        // Asked for again, it is the same picture, answered at once.
        let again = icons.request(&text, 32, false);
        assert_eq!(answers(&mut icons, &wakes, 1)[&again], Some(icon));

        // The item changed: another handle, and the old one names nothing.
        icons.forget(&text);
        assert!(icons.picture(icon).is_none());
        assert_eq!(icons.held_bytes(), 16 * 16 * 4);
        let anew = icons.request(&text, 32, false);
        let new = answers(&mut icons, &wakes, 1)[&anew].expect("a picture");
        assert_ne!(new, icon);
        assert!(icons.picture(new).is_some());
        assert!(icons.picture(thumbnail).is_some());
    }

    #[test]
    fn no_more_pixels_are_kept_than_the_bound_and_the_least_used_go_first() {
        let (mut icons, wakes) = pictures();
        // Each picture is 16 by 16: 1024 bytes. Room for three.
        const ONE: usize = 16 * 16 * 4;
        icons.set_budget(3 * ONE);
        let item = |number: usize| ItemId::new(format!("/folder/{number}.txt"));
        let mut handle_of = Vec::new();
        for number in 0..3 {
            let request = icons.request(&item(number), 16, false);
            handle_of.push(answers(&mut icons, &wakes, 1)[&request].expect("a picture"));
        }
        assert_eq!(icons.held_bytes(), 3 * ONE);
        // The renderer takes the second and then the third. A fourth
        // comes: of the two that were taken, the one taken longest ago
        // goes; the first, which nobody took yet, stays to be taken.
        assert!(icons.picture(handle_of[1]).is_some());
        assert!(icons.picture(handle_of[2]).is_some());
        let request = icons.request(&item(3), 16, false);
        let fourth = answers(&mut icons, &wakes, 1)[&request].expect("a picture");
        assert_eq!(icons.held_bytes(), 3 * ONE);
        assert!(
            icons.picture(handle_of[1]).is_none(),
            "the one taken longest ago was kept"
        );
        for kept in [handle_of[0], handle_of[2], fourth] {
            assert!(icons.picture(kept).is_some());
        }

        // What went is made again when asked for, under another handle.
        let request = icons.request(&item(1), 16, false);
        let again = answers(&mut icons, &wakes, 1)[&request].expect("a picture");
        assert_ne!(again, handle_of[1]);
        assert!(icons.picture(again).is_some());
        assert_eq!(icons.held_bytes(), 3 * ONE);

        // Thousands asked for one after the other and never taken hold
        // no more than the bound, however many there were: then the
        // oldest go.
        for number in 10..2010 {
            let request = icons.request(&item(number), 16, false);
            answers(&mut icons, &wakes, 1)[&request].expect("a picture");
            assert!(icons.held_bytes() <= 3 * ONE);
        }

        // What one drain answers is all there when it is told, even
        // where that is more than the bound; the next drain lets go.
        icons.set_budget(ONE);
        assert_eq!(icons.held_bytes(), ONE);
        let requests: Vec<IconRequest> = (5000..5004)
            .map(|number| icons.request(&item(number), 16, false))
            .collect();
        let mut told = HashMap::new();
        while told.len() < requests.len() {
            wakes.recv_timeout(Duration::from_secs(10)).expect("woken");
            for event in icons.drain() {
                let BackendEvent::IconReady {
                    request,
                    icon: Some(handle),
                } = event
                else {
                    panic!("a picture for each");
                };
                assert!(
                    icons.picture(handle).is_some(),
                    "told of a picture that is not there"
                );
                told.insert(request, handle);
            }
        }
        let _ = icons.drain();
        assert!(icons.held_bytes() <= ONE, "{}", icons.held_bytes());

        // A bound of nothing keeps nothing once the next drain came.
        icons.set_budget(0);
        assert_eq!(icons.held_bytes(), 0);
    }
}
