//! The screen is locked before the machine sleeps.
//!
//! logind lets a program hold sleep off for a moment (a "delay
//! inhibitor") and tells it when the machine is about to sleep. The shell
//! holds one, asks for the lock when sleep is announced, gives the lock a
//! moment to be on the screen, and lets go; after waking it holds one
//! again. Without logind, or when it refuses, nothing is locked before
//! sleep, as before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long the lock is given to be on the screen before sleep goes on.
/// (logind waits five seconds at most for those who hold it off.)
const FOR_THE_LOCK: Duration = Duration::from_millis(1500);

/// Start watching for sleep. `lock` is called (from a thread of this
/// module's own) when the machine is about to sleep and `wanted` is set.
pub fn watch(wanted: Arc<AtomicBool>, lock: impl Fn() + Send + 'static) -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::system()?;
    let manager = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    // Asked once here, so that a machine without logind says so now.
    let hold = |manager: &zbus::blocking::Proxy<'_>| -> zbus::Result<zbus::zvariant::OwnedFd> {
        manager.call(
            "Inhibit",
            &(
                "sleep",
                "gevrek",
                "The screen is locked before the machine sleeps",
                "delay",
            ),
        )
    };
    let mut held = Some(hold(&manager)?);
    log::debug!("sleep is held off until the screen is locked");
    let signals = manager.receive_signal("PrepareForSleep")?;
    std::thread::Builder::new()
        .name("gevrek-sleep".to_string())
        .spawn(move || {
            for message in signals {
                let going = match message.body().deserialize::<bool>() {
                    Ok(going) => going,
                    Err(error) => {
                        log::debug!("logind's PrepareForSleep was not understood: {error}");
                        continue;
                    }
                };
                if going {
                    if !wanted.load(Ordering::Relaxed) {
                        log::debug!("the machine sleeps, unlocked: the configuration wants no lock first");
                    }
                    if wanted.load(Ordering::Relaxed) {
                        log::info!("The machine is about to sleep: locking first");
                        lock();
                        std::thread::sleep(FOR_THE_LOCK);
                    }
                    // Let go: the machine sleeps.
                    drop(held.take());
                } else {
                    // Awake again: hold sleep off for the next time.
                    log::debug!("the machine is awake again");
                    held = hold(&manager)
                        .map_err(|error| log::debug!("logind did not hold sleep off: {error}"))
                        .ok();
                    if held.is_none() {
                        log::warn!("Sleep can no longer be held off for the lock");
                    }
                }
            }
        })?;
    Ok(())
}
