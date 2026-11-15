//! Asks git which commit is being built, for `gevrek_build::COMMIT`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    // Asked at every build (the file named here never exists, so the
    // script is never up to date): a change that is not committed shows as
    // one without anything of git's having changed. What depends on this
    // crate is only built again when the answer is another one.
    println!("cargo:rerun-if-changed=asked-at-every-build");
    println!("cargo:rerun-if-env-changed=GEVREK_COMMIT");
    // A package built from a source archive has no checkout to ask: who
    // builds it names the commit (`GEVREK_COMMIT`).
    let named = std::env::var("GEVREK_COMMIT")
        .ok()
        .map(|commit| commit.trim().chars().take(9).collect::<String>())
        .filter(|commit| !commit.is_empty());
    let commit = git(&["rev-parse", "--short=9", "HEAD"]).filter(|commit| !commit.is_empty());
    let stamp = match commit {
        _ if named.is_some() => named.unwrap_or_default(),
        // Built from a tree with changes that are in no commit.
        Some(commit) => match git(&["status", "--porcelain", "--untracked-files=no"]) {
            Some(changes) if !changes.is_empty() => format!("{commit}+changes"),
            _ => commit,
        },
        // Not built in a checkout (a source archive), or without git.
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=GEVREK_BUILD_COMMIT={stamp}");
}
