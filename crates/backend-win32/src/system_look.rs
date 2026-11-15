//! What Windows' own settings say about how the desktop should look, where
//! gevrek has a setting for the same thing: "Animation effects" switched
//! off (Settings, Accessibility, Visual effects) and a contrast theme in
//! use. Whoever sets them there means every program, gevrek too: each
//! counts as if gevrek's own setting were on (`desktop.animations.
//! reduce_motion`, `desktop.appearance.high_contrast`). gevrek's settings
//! stay what they are in the file; this is only what is carried out.
//!
//! (The shell and the settings program read the same two through
//! `gevrek_system::look`; the window manager does not use that crate.)

use gevrek_model::settings::{AnimationProfile, AppearanceProfile};
use std::sync::atomic::{AtomicBool, Ordering};

/// What the system said when it was last asked (`refresh`). Until then,
/// and in tests, nothing: gevrek's own settings alone count.
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);
static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);

/// Ask the system again. Returns whether either answer is another than
/// the last time (what follows them is then to be applied again).
pub fn refresh() -> bool {
    let (reduce_motion, high_contrast) = read();
    let was_reduce = REDUCE_MOTION.swap(reduce_motion, Ordering::Relaxed);
    let was_contrast = HIGH_CONTRAST.swap(high_contrast, Ordering::Relaxed);
    was_reduce != reduce_motion || was_contrast != high_contrast
}

/// The animations as they are carried out here.
pub fn animations(profile: &AnimationProfile) -> AnimationProfile {
    animations_with(profile, REDUCE_MOTION.load(Ordering::Relaxed))
}

/// The appearance as it is carried out here.
pub fn appearance(profile: &AppearanceProfile) -> AppearanceProfile {
    appearance_with(profile, HIGH_CONTRAST.load(Ordering::Relaxed))
}

fn animations_with(profile: &AnimationProfile, system_reduces: bool) -> AnimationProfile {
    let mut carried_out = profile.clone();
    carried_out.reduce_motion |= system_reduces;
    carried_out
}

fn appearance_with(profile: &AppearanceProfile, system_contrast: bool) -> AppearanceProfile {
    let mut carried_out = profile.clone();
    carried_out.high_contrast |= system_contrast;
    carried_out
}

/// The system's two settings now: movement is to be kept down, and a
/// contrast theme is in use. Where the system does not answer, neither.
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
    use super::*;

    #[test]
    fn the_systems_setting_counts_as_gevreks_own_and_never_takes_it_away() {
        let mut animations = AnimationProfile::default();
        assert!(!animations_with(&animations, false).reduce_motion);
        assert!(animations_with(&animations, true).reduce_motion);
        animations.reduce_motion = true;
        assert!(animations_with(&animations, false).reduce_motion);
        // Nothing else of the profile is touched.
        let carried_out = animations_with(&animations, true);
        assert_eq!(carried_out.enabled, animations.enabled);

        let mut appearance = AppearanceProfile::default();
        assert!(!appearance_with(&appearance, false).high_contrast);
        assert!(appearance_with(&appearance, true).high_contrast);
        appearance.high_contrast = true;
        assert!(appearance_with(&appearance, false).high_contrast);
        assert_eq!(
            appearance_with(&appearance, true).border_width,
            appearance.border_width
        );
    }

    #[test]
    fn the_system_answers_when_it_is_asked() {
        // Whatever this machine is set to: asking does not fail, and asking
        // twice in a row finds nothing changed.
        let _ = refresh();
        assert!(!refresh());
        REDUCE_MOTION.store(false, Ordering::Relaxed);
        HIGH_CONTRAST.store(false, Ordering::Relaxed);
    }
}
