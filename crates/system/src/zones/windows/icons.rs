//! Icons and small pictures of the shell's items.
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

use crate::icons::Picture;
use gevrek_zones::backend::IconSize;

/// The side of a picture in pixels: the logical size at the display's
/// scale, within what the shell draws.
pub fn pixels(size: IconSize) -> u32 {
    let scale = if size.scale.is_finite() && size.scale > 0.0 { size.scale } else { 1.0 };
    ((f64::from(size.size) * scale).round() as u32).clamp(8, 512)
}

/// The pictures of the shell's items: what is common to both systems
/// (`super::super::pictures`), with the shell asked for each.
pub type Icons = super::super::pictures::Pictures;

/// The pictures, made by the shell. `wake` is called when one was made:
/// the moment to call `drain` again. `programs` is what is known of the
/// programs that are items (which are no items of the shell's, and whose
/// pictures are found by what each program names).
pub fn of_shell(
    programs: std::sync::Arc<std::sync::Mutex<crate::zones::programs::Known>>,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
) -> Icons {
    Icons::with(
        move |id, size, thumbnail| {
            if crate::zones::smart::Smart::is_program(&gevrek_zones::ids::ItemId::new(id)) {
                // (A program holds no picture of what is in it.)
                if thumbnail {
                    return None;
                }
                let icon = programs.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).icon_of(id)?;
                return crate::zones::programs::picture(&icon, size);
            }
            look(id, size, thumbnail)
        },
        wake,
    )
}

/// Ask the shell for a picture of the item with this id.
///
/// A file that is not on this machine (one a cloud keeps, shown as a
/// placeholder) is never read for its picture: reading would fetch it.
/// Its icon is that of its kind, and its small picture only what Windows
/// already has in its own store. See `super::cloud`.
fn look(id: &str, size: u32, thumbnail: bool) -> Option<Picture> {
    let name = super::shell::plain(id);
    if super::cloud::state_of_path(&name).is_some_and(|state| state.fetched_when_read()) {
        return super::cloud::picture(&name, size, thumbnail);
    }
    crate::icons::shell::picture_as(&name, size, thumbnail)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
