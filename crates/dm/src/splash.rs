//! The boot splash (plymouth), where one is running when the service
//! starts: it has the display, and gevrek's login is to take it over
//! without anything else being seen in between.
//!
//! That is done as gdm does it. Before the greeter starts, the splash is
//! told to let go of the display (`plymouth deactivate`): it shows its
//! end, stops drawing and leaves its last picture there. gevrek's theme
//! ends in black (resources/plymouth), and black is what the login
//! begins with. When the greeter has drawn, the splash is ended with its
//! picture kept (`plymouth quit --retain-splash`): by then gevrek has
//! the display's device open, so that the kernel's own console is not
//! put back on the display when the splash closes it.
//!
//! The unit says the rest: the service takes the place of
//! `plymouth-quit.service` (which would end the splash with the text
//! console under it), and that one runs after all when the service fails.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A splash is there and has not been ended yet.
static THERE: AtomicBool = AtomicBool::new(false);

/// The splash's end (the theme's is two thirds of a second, after its
/// loop is through) is not waited for longer than this.
const TO_LET_GO: Duration = Duration::from_secs(6);
/// A greeter that has not said that it is shown after this long is not
/// waited for: the splash is ended all the same (the system counts the
/// start as done only then).
const GREETER_LATE: Duration = Duration::from_secs(20);

/// Run `plymouth` with these words; whether it said yes within `within`.
fn plymouth(words: &[&str], within: Duration) -> bool {
    let Ok(mut child) = Command::new("plymouth")
        .args(words)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        // (No plymouth on this system.)
        return false;
    };
    let since = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if since.elapsed() < within => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The words that end the splash: with its picture left on the display
/// (the login is there under it), or with the text console put back
/// (nothing of gevrek's is coming).
pub fn ending(keep_picture: bool) -> &'static [&'static str] {
    if keep_picture {
        &["quit", "--retain-splash"]
    } else {
        &["quit"]
    }
}

/// Before the first greeter: a splash that is running lets go of the
/// display. Comes back when it has (its end has been shown).
pub fn let_go() {
    if !plymouth(&["--ping"], Duration::from_secs(2)) {
        return;
    }
    THERE.store(true, Ordering::SeqCst);
    let since = Instant::now();
    if plymouth(&["deactivate"], TO_LET_GO) {
        log::info!(
            "the boot splash has let go of the display ({} ms)",
            since.elapsed().as_millis()
        );
    } else {
        log::warn!("the boot splash did not let go of the display");
    }
    std::thread::spawn(|| {
        std::thread::sleep(GREETER_LATE);
        if THERE.load(Ordering::SeqCst) {
            log::warn!(
                "the greeter did not say that it is shown: the boot splash is ended without it"
            );
            end(true);
        }
    });
}

/// End the splash, if there is one still (once: afterwards this does
/// nothing).
pub fn end(keep_picture: bool) {
    if !THERE.swap(false, Ordering::SeqCst) {
        return;
    }
    if plymouth(ending(keep_picture), Duration::from_secs(5)) {
        log::info!("the boot splash is ended");
    } else {
        log::warn!("the boot splash was not ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_splash_keeps_its_picture_only_for_a_login_that_is_there() {
        assert_eq!(ending(true), ["quit", "--retain-splash"]);
        assert_eq!(ending(false), ["quit"]);
    }

    #[test]
    fn without_a_splash_nothing_is_ended() {
        // (Nothing was found running: no program is started.)
        assert!(!THERE.load(Ordering::SeqCst));
        end(true);
        assert!(!THERE.load(Ordering::SeqCst));
    }
}
