//! What the system's own settings say about how things should look, where
//! gevrek has a setting for the same: movement kept down, and more
//! contrast.
//!
//! On Windows these are "Animation effects" switched off (Settings,
//! Accessibility, Visual effects) and a contrast theme in use; whoever
//! sets them there means every program, so gevrek's own programs count
//! each as if gevrek's setting were on. On Linux gevrek's settings are
//! the desktop's (applications are told of them through the portal);
//! gevrek's own programs ask that portal too, so that one started in
//! another desktop's session follows that desktop.
//!
//! (The Windows window manager reads the same two itself,
//! `gevrek_win32::system_look`: it does not use this crate.)

/// The system asks for less movement.
pub fn reduce_motion() -> bool {
    read().0
}

/// The system asks for more contrast.
pub fn high_contrast() -> bool {
    read().1
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read() -> (bool, bool) {
    (false, false)
}

/// On Linux: what the desktop's settings portal says
/// (`org.freedesktop.appearance`: `reduced-motion`, `contrast`). In
/// gevrek's own session that is gevrek's shell answering with gevrek's
/// settings; in another desktop's session (the studio started there) it
/// is that desktop's. Asked in the background and kept for ten seconds:
/// whoever asks gets the last answer at once (the first time: "no").
#[cfg(target_os = "linux")]
fn read() -> (bool, bool) {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static ANSWER: AtomicU8 = AtomicU8::new(0);
    static ASKING: AtomicBool = AtomicBool::new(false);
    static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
    let stale = {
        let asked = ASKED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        asked.is_none_or(|at| at.elapsed() >= Duration::from_secs(10))
    };
    if stale && !ASKING.swap(true, Ordering::SeqCst) {
        *ASKED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
        let ask = || {
            let answer = u8::from(portal_says("reduced-motion") == Some(1))
                | u8::from(portal_says("contrast") == Some(1)) << 1;
            ANSWER.store(answer, Ordering::SeqCst);
            ASKING.store(false, Ordering::SeqCst);
        };
        if std::thread::Builder::new()
            .name("gevrek-look".to_string())
            .spawn(ask)
            .is_err()
        {
            ASKING.store(false, Ordering::SeqCst);
        }
    }
    let answer = ANSWER.load(Ordering::SeqCst);
    (answer & 1 != 0, answer & 2 != 0)
}

/// One number of the settings portal's `org.freedesktop.appearance`, or
/// `None` when there is no portal, no such setting, or no answer in time.
#[cfg(target_os = "linux")]
fn portal_says(key: &str) -> Option<u32> {
    // (Without a session bus there is nobody to ask.)
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS")?;
    let output = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            "2",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            key,
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    portal_number(&String::from_utf8_lossy(&output.stdout))
}

/// The number in a portal's answer as `gdbus` prints it:
/// `(<<uint32 1>>,)`.
#[cfg(any(target_os = "linux", test))]
fn portal_number(answer: &str) -> Option<u32> {
    let rest = &answer[answer.find("uint32")? + "uint32".len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(windows)]
fn read() -> (bool, bool) {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    // SAFETY: each call fills in the value it is given a pointer to, of
    // the size the call is documented to write.
    unsafe {
        let mut animated = BOOL(1);
        let reduce_motion = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut animated).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && !animated.as_bool();
        let size = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        let mut contrast = HIGHCONTRASTW {
            cbSize: size,
            ..Default::default()
        };
        let high_contrast = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            size,
            Some(std::ptr::from_mut(&mut contrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && contrast.dwFlags.contains(HCF_HIGHCONTRASTON);
        (reduce_motion, high_contrast)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_number_in_a_portals_answer_is_read() {
        assert_eq!(super::portal_number("(<<uint32 1>>,)\n"), Some(1));
        assert_eq!(super::portal_number("(<<uint32 0>>,)"), Some(0));
        assert_eq!(super::portal_number("(<uint32 2>,)"), Some(2));
        assert_eq!(super::portal_number("(<<'dark'>>,)"), None);
        assert_eq!(super::portal_number(""), None);
    }

    #[test]
    fn the_system_answers_when_it_is_asked() {
        // Whatever this machine is set to: asking does not fail, and the
        // answer is the same when asked again at once.
        assert_eq!(super::read(), super::read());
    }
}
