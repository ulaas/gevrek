//! What the compositor has to tell the person at the machine (the
//! engine's [`Notice`]): settings it reads and cannot follow. A notice is
//! there while its cause is; the panel shows a mark for it, and `gevrek
//! health` and `gevrek state --json` list it.

use gevrek_engine::overlay_data::Notice;
use gevrek_model::outputs::{profile_of, OutputProfile};

/// What `desktop.outputs` asks of the displays that are there and could
/// not be done: a refresh rate that follows the program, on a display
/// (or with a driver, or in a session) that cannot. `follows` says of a
/// display whether its rate does follow. Saying so here is better than a
/// display that silently stays as it was.
pub fn of_outputs(
    profiles: &[OutputProfile],
    connected: &[&str],
    follows: impl Fn(&str) -> bool,
) -> Vec<Notice> {
    let mut notices = Vec::new();
    for name in connected {
        let Some(profile) = profile_of(profiles, name).filter(|profile| profile.enabled) else {
            continue;
        };
        let key = name.trim().to_lowercase();
        if profile.variable_refresh && !follows(name) {
            notices.push(Notice {
                id: format!("output-variable-refresh-{key}"),
                title: format!("{name} keeps a fixed refresh rate"),
                text: format!(
                    "The configuration asks for a refresh rate on {name} that follows the \
                     program (desktop.outputs, variable_refresh). The display, its driver or \
                     this kind of session cannot do that: the display keeps the rate of its \
                     mode. Switch variable_refresh off to put this notice away."
                ),
            });
        }
    }
    notices
}

/// The night light is asked for and cannot be shown here.
pub fn of_night_light(
    night_light: &gevrek_model::night_light::NightLightConfig,
    impossible: bool,
) -> Option<Notice> {
    (night_light.enabled && impossible).then(|| Notice {
        id: "night-light".to_string(),
        title: "The night light cannot be shown".to_string(),
        text: "The configuration asks for warmer colours in the evening \
               (desktop.night_light). No display of this session takes the colour ramp \
               that needs: a session in a window or without real displays cannot, and \
               some display drivers cannot. Switch it off to put this notice away."
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> OutputProfile {
        OutputProfile {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_setting_of_a_display_that_is_not_followed_is_told_while_the_display_is_there() {
        use gevrek_model::outputs::OutputTransform;
        let mut turned = profile("dp-1");
        turned.transform = OutputTransform::Right;
        turned.variable_refresh = true;
        let mut away = profile("HDMI-A-1");
        away.variable_refresh = true;
        let mut off = profile("DP-3");
        off.enabled = false;
        off.variable_refresh = true;
        let profiles = [turned, away, off, profile("eDP-1")];

        let notices = of_outputs(&profiles, &["DP-1", "DP-3", "eDP-1"], |_| false);
        let ids: Vec<&str> = notices.iter().map(|notice| notice.id.as_str()).collect();
        // The display by the name it has here, whatever the file's case;
        // not the one that is not plugged in, nor the one switched off.
        // (Turning the picture is done, and is nothing to tell of.)
        assert_eq!(ids, ["output-variable-refresh-dp-1"]);
        assert!(notices[0].title.contains("DP-1"));
        assert!(notices[0].text.contains("variable_refresh"));

        // Nothing asked that is not done: nothing to tell.
        assert!(of_outputs(&[profile("DP-1")], &["DP-1"], |_| false).is_empty());
        assert!(of_outputs(&[], &["DP-1"], |_| false).is_empty());
        // Asked for and done: nothing to tell either.
        assert!(
            of_outputs(&profiles, &["DP-1", "DP-3", "eDP-1"], |name| name == "DP-1").is_empty()
        );
    }
}
