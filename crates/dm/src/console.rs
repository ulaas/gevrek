//! The virtual consoles: which is shown, and whether the kernel's own
//! text console may paint on one.

use std::os::fd::AsRawFd;

const VT_ACTIVATE: libc::c_ulong = 0x5606;
const VT_WAITACTIVE: libc::c_ulong = 0x5607;
const KDSETMODE: libc::c_ulong = 0x4B3A;
const KD_TEXT: libc::c_ulong = 0;
const KD_GRAPHICS: libc::c_ulong = 1;

fn open(console: u32) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/tty{console}"))
}

fn ioctl(
    file: &std::fs::File,
    request: libc::c_ulong,
    value: libc::c_ulong,
) -> std::io::Result<()> {
    // SAFETY: the requests named here take a plain number.
    let done = unsafe { libc::ioctl(file.as_raw_fd(), request, value) };
    if done < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A console held for a session: while it is open the console counts as
/// taken (logind starts no text login on it when it is shown), and the
/// kernel's own text console does not paint on it (a compositor is going
/// to draw there; what was shown before stays until its first frame).
pub struct Held {
    file: std::fs::File,
}

impl Held {
    pub fn take(console: u32) -> std::io::Result<Self> {
        let file = open(console)?;
        ioctl(&file, KDSETMODE, KD_GRAPHICS)?;
        Ok(Self { file })
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // The console is a text console again for whoever has it next.
        if let Err(error) = ioctl(&self.file, KDSETMODE, KD_TEXT) {
            log::debug!("a console was not made a text console again: {error}");
        }
    }
}

/// A console is a text console again (after a worker that could not
/// say so itself any more).
pub fn release(console: u32) {
    match open(console).and_then(|file| ioctl(&file, KDSETMODE, KD_TEXT)) {
        Ok(()) => log::debug!("console {console} is a text console again"),
        Err(error) => log::debug!("console {console} was not made a text console again: {error}"),
    }
}

/// Show this console, and wait until it is the one shown.
pub fn show(console: u32) -> std::io::Result<()> {
    log::debug!("console {console} is to be shown");
    let file = open(0)?;
    ioctl(&file, VT_ACTIVATE, libc::c_ulong::from(console))?;
    ioctl(&file, VT_WAITACTIVE, libc::c_ulong::from(console))
}
