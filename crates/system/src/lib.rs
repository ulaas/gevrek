//! What the shell asks of the system.
//!
//! Each thing the shell needs from outside gevrek (which applications are
//! installed, later their icons, the sound, the battery) has one module
//! here with one face for every system, and behind it the system's own
//! way: desktop entries on Linux, the Start menu on Windows.
//!
//! So far: [`apps`] (with [`start_menu`] behind it on Windows), [`icons`],
//! [`sound`], [`session`] (its ends and the displays switched off, on
//! Windows), [`idle`] (the system's own times, on Windows), [`status`]
//! (the battery and the network), and [`every_key`] (the keyboard for one
//! program while a binding's keys are recorded, on Windows).

pub mod apps;
pub mod bluetooth;
pub mod brightness;
pub mod clipboard;
#[cfg(windows)]
pub mod display_ids;
pub mod every_key;
pub mod icons;
pub mod idle;
pub mod look;
pub mod pictures;
pub mod playing;
pub mod power_profile;
pub mod session;
pub mod sound;
#[cfg(target_os = "linux")]
pub mod sound_server;
pub mod start_menu;
pub mod status;
pub mod tray_host;
pub mod wifi;
pub mod zones;

/// Windows' way of letting a thread talk to the shell's objects.
#[cfg(windows)]
pub(crate) mod com {
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };

    /// The thread may use the shell's objects while this lives. A thread
    /// that was set up already, in whichever way, stays as it is.
    pub struct Com(bool);

    impl Com {
        pub fn new() -> Self {
            let asked =
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            // Every time it says yes (also "it was already"), it wants to
            // be told once that we are done.
            Self(asked.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }
}
