//! The machine's power profile: saving power, balanced, or all it can
//! do. Kept by the system's power-profiles service (`power-profiles-daemon`
//! or `tuned-ppd`, both under the name `net.hadess.PowerProfiles` on the
//! system's bus), which is asked with `busctl`. A machine without that
//! service has no profiles to choose from.

/// The profiles the machine has, and which is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    pub active: String,
    pub all: Vec<String>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SERVICE: [&str; 3] = [
    "net.hadess.PowerProfiles",
    "/net/hadess/PowerProfiles",
    "net.hadess.PowerProfiles",
];

/// The profile in use, out of `busctl`'s answer (`s "balanced"`).
pub fn active_in(answer: &str) -> Option<String> {
    let rest = answer.trim().strip_prefix("s ")?;
    let name = rest.trim().trim_matches('"');
    (!name.is_empty()).then(|| name.to_string())
}

/// The profiles there are, out of `busctl`'s answer: every text that
/// follows `"Profile" s`, in the order they stand.
pub fn all_in(answer: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = answer;
    while let Some(at) = rest.find("\"Profile\" s \"") {
        rest = &rest[at + "\"Profile\" s \"".len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let name = &rest[..end];
        if !name.is_empty() && !found.iter().any(|known| known == name) {
            found.push(name.to_string());
        }
        rest = &rest[end..];
    }
    found
}

/// What a profile is called for a person.
pub fn label(profile: &str) -> &str {
    match profile {
        "power-saver" => "Saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        other => other,
    }
}

/// A name that may be handed to the service: one of its own kind
/// (letters, digits and `-`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_a_name(profile: &str) -> bool {
    !profile.is_empty()
        && profile.len() <= 32
        && profile
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '-')
}

#[cfg(target_os = "linux")]
fn ask(arguments: &[&str]) -> Option<String> {
    let output = std::process::Command::new("busctl")
        .arg("--system")
        .args(arguments)
        .output()
        .map_err(|error| log::debug!("busctl did not start: {error}"))
        .ok()?;
    if !output.status.success() {
        log::debug!("busctl {}: {}", arguments.join(" "), output.status);
    }
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The machine's profiles; `None`: it has no service that keeps them.
#[cfg(target_os = "linux")]
pub fn read() -> Option<Profiles> {
    let [name, path, interface] = SERVICE;
    let active = active_in(&ask(&[
        "get-property",
        name,
        path,
        interface,
        "ActiveProfile",
    ])?)?;
    let all = all_in(&ask(&["get-property", name, path, interface, "Profiles"])?);
    (!all.is_empty()).then_some(Profiles { active, all })
}

/// Make this the profile in use. Whether the service took it.
#[cfg(target_os = "linux")]
pub fn choose(profile: &str) -> bool {
    let [name, path, interface] = SERVICE;
    let took = is_a_name(profile)
        && ask(&[
            "set-property",
            name,
            path,
            interface,
            "ActiveProfile",
            "s",
            profile,
        ])
        .is_some();
    log::debug!(
        "the power profile {profile:?}: {}",
        if took { "chosen" } else { "not taken" }
    );
    took
}

/// Chosen in the system's own settings here.
#[cfg(not(target_os = "linux"))]
pub fn read() -> Option<Profiles> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn choose(_profile: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_services_answers_are_read() {
        assert_eq!(active_in("s \"balanced\"\n").as_deref(), Some("balanced"));
        assert_eq!(active_in("s \"\""), None);
        assert_eq!(active_in("b true"), None);
        let all = "aa{sv} 3 2 \"Profile\" s \"power-saver\" \"Driver\" s \"tuned\" 2 \"Profile\" s \"balanced\" \"Driver\" s \"tuned\" 2 \"Profile\" s \"performance\" \"Driver\" s \"tuned\"\n";
        assert_eq!(all_in(all), ["power-saver", "balanced", "performance"]);
        // The driver first, as the other service writes it; one named twice once.
        let other = "aa{sv} 2 3 \"Driver\" s \"intel_pstate\" \"Profile\" s \"balanced\" \"CpuDriver\" s \"x\" 2 \"Profile\" s \"balanced\" \"Driver\" s \"y\"";
        assert_eq!(all_in(other), ["balanced"]);
        assert!(all_in("aa{sv} 0").is_empty());
    }

    #[test]
    fn a_profile_has_a_word_for_a_person_and_a_name_for_the_service() {
        assert_eq!(label("power-saver"), "Saver");
        assert_eq!(label("performance"), "Performance");
        assert_eq!(label("quiet"), "quiet");
        assert!(is_a_name("power-saver"));
        assert!(!is_a_name(""));
        assert!(!is_a_name("x y"));
        assert!(!is_a_name("a\"; b"));
    }
}
