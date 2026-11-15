//! A pointer and a keyboard made up for an application that was let
//! control the desktop (the RemoteDesktop portal).
//!
//! What such an application sends through the portal is passed on to
//! the compositor as any program of the session may do it: a pointer
//! of `wlr-virtual-pointer` and a keyboard of `virtual-keyboard`, on a
//! connection of the shell's own. The keyboard has the session's own
//! keymap (what `XKB_DEFAULT_*` says, else `us`); a key told by what it
//! types (a keysym) is looked up in it.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::Instant;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1 as KeyboardManager,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1 as Keyboard,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 as PointerManager,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 as Pointer,
};
use xkbcommon::xkb;

/// What an application that controls the desktop does.
#[derive(Debug, Clone, PartialEq)]
pub enum Deed {
    /// The pointer moved by so much.
    Motion(f64, f64),
    /// The pointer put at a place of a display: the display's name, the
    /// place, and the size the place is counted in.
    Place {
        output: String,
        at: (f64, f64),
        size: (u32, u32),
    },
    /// A button (as Linux numbers them: 272 the left) down or up.
    Button(u32, bool),
    /// The wheel turned by so much, sideways and down.
    Turn(f64, f64),
    /// The wheel turned by whole notches: on the axis (0 down, 1
    /// sideways), so many.
    Notches(u32, i32),
    /// A key by its place on the keyboard (as Linux numbers them) down
    /// or up.
    Key(u32, bool),
    /// A key by what it types, down or up.
    Sym(u32, bool),
}

#[derive(Default)]
struct Heard {
    /// The displays: their objects, and the names they were told with.
    outputs: Vec<(wl_output::WlOutput, String)>,
}

/// The connection, the made-up devices and the keyboard's state.
pub struct Remote {
    connection: Connection,
    queue: EventQueue<Heard>,
    heard: Heard,
    seat: wl_seat::WlSeat,
    pointers: PointerManager,
    pointer: Pointer,
    /// A pointer of each display it was put on a place of.
    placed: Vec<(String, Pointer)>,
    keyboard: Keyboard,
    keymap: xkb::Keymap,
    keys: xkb::State,
    began: Instant,
}

/// The key that types a keysym in a keymap's first layout, and whether
/// Shift is to be held with it. `None` where no key types it.
pub fn key_for(keymap: &xkb::Keymap, sym: u32) -> Option<(u32, bool)> {
    let (first, last) = (keymap.min_keycode().raw(), keymap.max_keycode().raw());
    for level in 0..2 {
        for code in first..=last {
            let typed = keymap.key_get_syms_by_level(xkb::Keycode::new(code), 0, level);
            if typed.iter().any(|typed| typed.raw() == sym) {
                // (XKB's numbers are Linux's plus 8.)
                return Some((code.checked_sub(8)?, level == 1));
            }
        }
    }
    None
}

/// The session's keymap, as its text.
fn session_keymap() -> anyhow::Result<xkb::Keymap> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // (Empty names: what the environment says, else XKB's defaults.)
    xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
        .ok_or_else(|| anyhow::anyhow!("the session's keymap could not be made"))
}

/// A file in memory with this text in it.
fn in_memory(text: &str) -> anyhow::Result<OwnedFd> {
    use std::io::Write;
    // SAFETY: memfd_create with a constant name; the descriptor it
    // returns is owned by nothing else.
    let raw = unsafe { libc::memfd_create(c"gevrek-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: just made, valid, and owned here alone.
    let file = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut writer = std::fs::File::from(file.try_clone()?);
    writer.write_all(text.as_bytes())?;
    writer.write_all(&[0])?;
    Ok(file)
}

impl Remote {
    /// Make the pointer and the keyboard. An error when the compositor
    /// does not let programs make them up.
    pub fn connect() -> anyhow::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<Heard>(&connection)?;
        let handle = queue.handle();
        let seat: wl_seat::WlSeat = globals
            .bind(&handle, 1..=5, ())
            .map_err(|error| anyhow::anyhow!("there is no seat: {error}"))?;
        let pointers: PointerManager = globals
            .bind(&handle, 1..=2, ())
            .map_err(|error| anyhow::anyhow!("no pointer can be made up: {error}"))?;
        let keyboards: KeyboardManager = globals
            .bind(&handle, 1..=1, ())
            .map_err(|error| anyhow::anyhow!("no keyboard can be made up: {error}"))?;
        let mut heard = Heard::default();
        for global in globals.contents().clone_list() {
            if global.interface == wl_output::WlOutput::interface().name && global.version >= 4 {
                let output: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(global.name, 4, &handle, heard.outputs.len());
                heard.outputs.push((output, String::new()));
            }
        }
        let pointer = pointers.create_virtual_pointer(Some(&seat), &handle, ());
        let keyboard = keyboards.create_virtual_keyboard(&seat, &handle, ());
        let keymap = session_keymap()?;
        let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        let file = in_memory(&text)?;
        keyboard.keymap(
            wl_keyboard::KeymapFormat::XkbV1 as u32,
            file.as_fd(),
            text.len() as u32 + 1,
        );
        queue.roundtrip(&mut heard)?;
        let keys = xkb::State::new(&keymap);
        Ok(Self {
            connection,
            queue,
            heard,
            seat,
            pointers,
            pointer,
            placed: Vec::new(),
            keyboard,
            keymap,
            keys,
            began: Instant::now(),
        })
    }

    fn time(&self) -> u32 {
        self.began.elapsed().as_millis() as u32
    }

    /// A key by its place, down or up; the compositor is told what is
    /// held from then on.
    fn key(&mut self, code: u32, pressed: bool) {
        let time = self.time();
        self.keyboard.key(time, code, u32::from(pressed));
        self.keys.update_key(
            xkb::Keycode::new(code + 8),
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        self.keyboard.modifiers(
            self.keys.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.keys.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.keys.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.keys.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        );
    }

    /// Do what the application did.
    pub fn act(&mut self, deed: &Deed) -> anyhow::Result<()> {
        let time = self.time();
        match deed {
            Deed::Motion(dx, dy) => {
                self.pointer.motion(time, *dx, *dy);
                self.pointer.frame();
            }
            Deed::Place { output, at, size } => {
                let known = self.placed.iter().position(|(name, _)| name == output);
                let index = match known {
                    Some(index) => index,
                    None => {
                        let Some((display, _)) =
                            self.heard.outputs.iter().find(|(_, name)| name == output)
                        else {
                            anyhow::bail!("there is no display {output}");
                        };
                        let pointer = self.pointers.create_virtual_pointer_with_output(
                            Some(&self.seat),
                            Some(display),
                            &self.queue.handle(),
                            (),
                        );
                        self.placed.push((output.clone(), pointer));
                        self.placed.len() - 1
                    }
                };
                let pointer = &self.placed[index].1;
                let within = |value: f64, size: u32| value.clamp(0.0, f64::from(size)) as u32;
                pointer.motion_absolute(
                    time,
                    within(at.0, size.0),
                    within(at.1, size.1),
                    size.0.max(1),
                    size.1.max(1),
                );
                pointer.frame();
            }
            Deed::Button(button, pressed) => {
                self.pointer.button(
                    time,
                    *button,
                    if *pressed {
                        wl_pointer::ButtonState::Pressed
                    } else {
                        wl_pointer::ButtonState::Released
                    },
                );
                self.pointer.frame();
            }
            Deed::Turn(dx, dy) => {
                self.pointer.axis_source(wl_pointer::AxisSource::Finger);
                if *dy != 0.0 {
                    self.pointer
                        .axis(time, wl_pointer::Axis::VerticalScroll, *dy);
                }
                if *dx != 0.0 {
                    self.pointer
                        .axis(time, wl_pointer::Axis::HorizontalScroll, *dx);
                }
                self.pointer.frame();
            }
            Deed::Notches(axis, steps) => {
                let axis = if *axis == 1 {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    wl_pointer::Axis::VerticalScroll
                };
                self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
                // (A notch of a wheel is fifteen of the pointer's units.)
                self.pointer
                    .axis_discrete(time, axis, f64::from(*steps) * 15.0, *steps);
                self.pointer.frame();
            }
            Deed::Key(code, pressed) => self.key(*code, *pressed),
            Deed::Sym(sym, pressed) => {
                let Some((code, shifted)) = key_for(&self.keymap, *sym) else {
                    anyhow::bail!("no key of the keyboard types {sym:#x}");
                };
                // (Shift_L, as Linux numbers it.)
                const SHIFT: u32 = 42;
                if shifted && *pressed {
                    self.key(SHIFT, true);
                }
                self.key(code, *pressed);
                if shifted && !*pressed {
                    self.key(SHIFT, false);
                }
            }
        }
        // (Sent now, and what the compositor says in between is read.)
        self.queue.roundtrip(&mut self.heard)?;
        self.connection.flush()?;
        Ok(())
    }
}

/// Make the devices on a thread of their own, which does what it is
/// sent for as long as somebody may send. An error when the devices
/// cannot be made.
pub fn spawn() -> anyhow::Result<std::sync::mpsc::Sender<Deed>> {
    let (deeds, todo) = std::sync::mpsc::channel::<Deed>();
    let (made, is_made) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("remote".to_string())
        .spawn(move || {
            let mut remote = match Remote::connect() {
                Ok(remote) => remote,
                Err(error) => {
                    let _ = made.send(Err(format!("{error:#}")));
                    return;
                }
            };
            let _ = made.send(Ok(()));
            for deed in todo {
                if let Err(error) = remote.act(&deed) {
                    log::warn!("what a controlling application did was not done: {error:#}");
                }
            }
        })?;
    match is_made.recv() {
        Ok(Ok(())) => Ok(deeds),
        Ok(Err(error)) => anyhow::bail!(error),
        Err(_) => anyhow::bail!("the thread of the made-up devices ended"),
    }
}

impl Dispatch<wl_output::WlOutput, usize> for Heard {
    fn event(
        heard: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(output) = heard.outputs.get_mut(*index) {
                output.1 = name;
            }
        }
    }
}

macro_rules! quiet {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Heard {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
quiet!(wl_seat::WlSeat, ());
quiet!(PointerManager, ());
quiet!(Pointer, ());
quiet!(KeyboardManager, ());
quiet!(Keyboard, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_found_by_what_it_types() {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            // (No XKB data on this machine: nothing to look in.)
            return;
        };
        // a: the key Linux numbers 30, alone; A: the same with Shift.
        assert_eq!(key_for(&keymap, 0x61), Some((30, false)));
        assert_eq!(key_for(&keymap, 0x41), Some((30, true)));
        // Return, and the digit 1 with its mark.
        assert_eq!(key_for(&keymap, 0xff0d), Some((28, false)));
        assert_eq!(key_for(&keymap, 0x31), Some((2, false)));
        assert_eq!(key_for(&keymap, 0x21), Some((2, true)));
        // Nothing on a US keyboard types a Turkish ğ.
        assert_eq!(key_for(&keymap, 0x2bb), None);
    }
}
