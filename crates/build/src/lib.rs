//! Which commit this program was built from, so that whoever looks at a
//! running gevrek knows what is running: the list of shortcuts shows it,
//! and `gevrek health` says it of the window manager.

/// The commit the build was made from, as git abbreviates it, followed by
/// `+changes` when the tree had changes that were in no commit; `unknown`
/// where the build was not made in a checkout.
pub const COMMIT: &str = env!("GEVREK_BUILD_COMMIT");

/// The version with the commit: `0.6.0 (abd328412)`.
pub fn version() -> String {
    format!("{} ({COMMIT})", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_commit_is_known_or_said_not_to_be() {
        let commit = super::COMMIT.trim_end_matches("+changes");
        assert!(
            commit == "unknown"
                || (commit.len() >= 7 && commit.chars().all(|c| c.is_ascii_hexdigit())),
            "{commit}"
        );
        assert!(super::version().contains(super::COMMIT));
    }
}
