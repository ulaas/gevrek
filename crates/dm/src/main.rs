//! gevrek-dm: gevrek's display manager.
//!
//! A system service that runs as root and draws nothing. It starts the
//! greeter (gevrek's compositor with the login in it, as a user of its
//! own) on the first console, checks the password the greeter sends,
//! opens the user's session on the second console and switches to it;
//! when the session ends or dies it switches back to the greeter, which
//! was never ended. A boot splash that is running is taken over from
//! without a seam (`splash`). It starts gevrek and nothing else: no list of
//! sessions, no other desktop, no X11. The plan's M8 section has why it
//! is made this way.
//!
//!     gevrek-dm             the service (as root, from its unit)
//!     gevrek-dm --worker    one session's worker (started by the service)

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gevrek-dm is for Linux, where gevrek is the session a user logs into.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod console;
#[cfg(target_os = "linux")]
mod pam_session;
#[cfg(target_os = "linux")]
mod remember;
#[cfg(target_os = "linux")]
mod service;
#[cfg(target_os = "linux")]
mod splash;
#[cfg(target_os = "linux")]
mod worker;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    // (A service of the system: no person's configuration is its own, so
    // the build and the version say.)
    gevrek_logging::Logging::new("gevrek-dm", log::LevelFilter::Info)
        .without_setting()
        .init();
    match std::env::args().nth(1).as_deref() {
        Some("--worker") => worker::run(),
        None => service::run(),
        Some(other) => anyhow::bail!("gevrek-dm takes no argument but --worker (not {other})"),
    }
}
