//! Settings that only the Wayland backend can apply.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WaylandPlatformConfig {
    /// Run X11 programs through Xwayland. A change holds from the next
    /// start of the session (Xwayland is started with it, and ending it
    /// would end every X11 program that runs).
    pub xwayland: bool,
    /// The cursor theme and size programs are told to use
    /// (`XCURSOR_THEME`, `XCURSOR_SIZE`); empty or 0 leaves them alone.
    /// gevrek's own pointer changes at once; a program that runs already
    /// keeps the pointer it was started with.
    pub cursor_theme: String,
    pub cursor_size: u32,
    /// How many device pixels one logical pixel is, for every output
    /// (2 on a high-density display). Between 0.5 and 4.
    pub output_scale: f64,
    /// The scale of single outputs, by the name of their connector
    /// (`eDP-1`, `HDMI-A-1`); the others have `output_scale`.
    pub output_scales: std::collections::BTreeMap<String, f64>,
    /// A window that is drawn see-through (a rule's or a command's
    /// opacity) shows what lies behind it out of focus. Off: sharp, as
    /// it is behind glass that is clear.
    pub blur_behind_windows: bool,
    /// A display that is shared with another program (screen sharing)
    /// goes to it in buffers of the graphics card's own memory, with
    /// nothing copied on the way. Off unless asked for: a program that
    /// cannot take such buffers gets no pictures then.
    pub share_card_buffers: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl WaylandPlatformConfig {
    /// The scale of the output with this name, kept within what makes
    /// sense (0.5 to 4; anything else counts as 1).
    pub fn scale_of(&self, output: &str) -> f64 {
        let scale = self
            .output_scales
            .get(output)
            .copied()
            .unwrap_or(self.output_scale);
        if (0.5..=4.0).contains(&scale) {
            scale
        } else {
            1.0
        }
    }
}

impl Default for WaylandPlatformConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            xwayland: true,
            cursor_theme: String::new(),
            cursor_size: 0,
            output_scale: 1.0,
            output_scales: Default::default(),
            blur_behind_windows: true,
            share_card_buffers: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_has_its_own_scale_or_the_common_one() {
        let mut config: WaylandPlatformConfig = serde_json::from_str(
            r#"{"output_scale": 1.5, "output_scales": {"eDP-1": 2.0, "odd": 9.0}}"#,
        )
        .expect("configuration");
        assert_eq!(config.scale_of("eDP-1"), 2.0);
        assert_eq!(config.scale_of("HDMI-A-1"), 1.5);
        // Out of range counts as 1.
        assert_eq!(config.scale_of("odd"), 1.0);
        config.output_scale = 0.0;
        assert_eq!(config.scale_of("HDMI-A-1"), 1.0);
        // A configuration written before the setting existed has none.
        let old: WaylandPlatformConfig =
            serde_json::from_str(r#"{"output_scale": 2.0}"#).expect("configuration");
        assert!(old.output_scales.is_empty());
        assert_eq!(old.scale_of("anything"), 2.0);
    }
}
