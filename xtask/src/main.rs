//! Tasks of the repository, run as `cargo xtask <task>`:
//!
//! - `check`: everything that must hold before a commit. The tests of the
//!   workspace for this system, the type-check of the other system's side
//!   where its target is installed, and on Linux the end-to-end checks
//!   without a display. Stops at the first step that fails.
//! - `install [--prefix <dir>] [--debug] [--session]`: build and copy the
//!   programs to `<prefix>/bin` (default `~/.local`). The session you are
//!   logged into runs these, never the build in `target/` you are working
//!   on. The build installed before is kept as
//!   `<prefix>/lib/gevrek/previous`. On Windows, where the helper for
//!   administrator windows is registered, it is registered anew for this
//!   build (`keep_elevated_helper`). With `--session` (Linux) the files of
//!   a login session are installed too (`resources/README.md`), and the
//!   two commands that need root are printed.
//! - `uninstall [--prefix <dir>]`: remove what `install` put there.
//! - `package [--debug]` (Windows): the programs with the scripts that
//!   install, update and remove them for one person, in one zip
//!   (`target/package`): what is given to a machine without the sources.
//! - `rpm [--debug]` (Linux): the programs as an RPM, for the machine they
//!   were built on.
//! - `dist [kinds] [--debug]`: what a release offers, under `target/dist`:
//!   the zip and the setup program on Windows; the tarball, the .deb and
//!   the .rpm on Linux. The release workflow runs it.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The programs of this system.
fn programs() -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
            "gevrek",
            "gevrek-wm",
            // The helper for the windows of programs that run as an
            // administrator.
            "gevrek-elevated",
            "gevrek-studio",
            "gevrek-shell",
        ]
    } else {
        vec![
            "gevrek",
            "gevrek-wl",
            "gevrek-studio",
            "gevrek-shell",
            "gevrek-lock",
        ]
    }
}

/// Programs an earlier install put there and this one no longer does: the
/// old editor, which the studio replaced (it is no longer in the tree).
/// `install` and `uninstall` take the copies away, so that no old build
/// stays behind.
const RETIRED: &[&str] = &["gevrek-editor", "gevrek-overlay"];

/// Units of the session an earlier install put there and this one no
/// longer has: the bar of before the shell.
const RETIRED_UNITS: &[&str] = &["gevrek-bar.service"];

fn remove_retired(bin: &Path) -> Result<()> {
    for name in RETIRED {
        let file = bin.join(program_file(name));
        if file.exists() {
            std::fs::remove_file(&file)?;
            println!("removed {} (no longer installed)", file.display());
        }
    }
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is in the repository")
        .to_path_buf()
}

/// Where cargo builds: `target` in the repository, unless `CARGO_TARGET_DIR`
/// names another folder (a build inside WSL keeps it on the Linux side).
fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .unwrap_or_else(|| root().join("target"))
}

fn run(step: &str, command: &mut Command) -> Result<()> {
    println!("== {step}");
    let status = command
        .current_dir(root())
        .status()
        .with_context(|| format!("could not start: {step}"))?;
    if !status.success() {
        bail!("{step} failed");
    }
    Ok(())
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
}

/// Whether the standard library of a target is installed (for a check of
/// the other system's code without building or running it).
fn has_target(target: &str) -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.trim() == target)
        })
        .unwrap_or(false)
}

fn check() -> Result<()> {
    if cfg!(windows) {
        run(
            "tests (everything)",
            cargo().args(["test", "--workspace", "--all-targets", "--no-fail-fast"]),
        )?;
        // As CI does: warnings pass, an error (a lint that denies) does not.
        run(
            "clippy (no errors)",
            cargo().args(["clippy", "--workspace", "--all-targets"]),
        )?;
    } else {
        run(
            "tests (everything but the Windows backend)",
            cargo().args([
                "test",
                "--workspace",
                "--exclude",
                "gevrek-win32",
                "--no-fail-fast",
            ]),
        )?;
        if has_target("x86_64-pc-windows-msvc") {
            run(
                "type-check of the Windows side",
                cargo()
                    .args(["check", "--target", "x86_64-pc-windows-msvc", "--workspace"])
                    .args([
                        "--exclude",
                        "gevrek-smithay",
                        "--exclude",
                        "xtask",
                        "--all-targets",
                    ])
                    .env("CARGO_TARGET_DIR", target_dir().join("win")),
            )?;
        } else {
            println!("== type-check of the Windows side: skipped (rustup target add x86_64-pc-windows-msvc)");
        }
        run(
            "build for the nested checks",
            cargo().args([
                "build",
                "-p",
                "gevrek-smithay",
                "-p",
                "gevrek",
                "-p",
                "gevrek-testclient",
                "-p",
                "gevrek-studio",
                "-p",
                "gevrek-shell",
                "-p",
                "gevrek-lock",
            ]),
        )?;
        run(
            "build of the design-system gallery",
            cargo().args(["build", "-p", "gevrek-ui", "--example", "gallery"]),
        )?;
        run(
            "build of the surface host's spike",
            cargo().args(["build", "-p", "gevrek-shellkit", "--example", "spike"]),
        )?;
        // (Where they are: not every copy of the repository has them.)
        if std::path::Path::new("tests/nested/checks.py").is_file() {
            run(
                "nested checks (no display)",
                Command::new("python3").arg("tests/nested/checks.py"),
            )?;
        }
    }
    println!("== all checks passed");
    Ok(())
}

fn home() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("no home folder")
}

fn prefix(arguments: &[String]) -> Result<PathBuf> {
    match arguments.iter().position(|argument| argument == "--prefix") {
        Some(index) => arguments
            .get(index + 1)
            .map(PathBuf::from)
            .context("--prefix needs a folder"),
        None => Ok(home()?.join(".local")),
    }
}

fn program_file(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn install(arguments: &[String]) -> Result<()> {
    let prefix = prefix(arguments)?;
    let debug = arguments.iter().any(|argument| argument == "--debug");
    let mut build = cargo();
    build.arg("build").arg("--bins");
    if !debug {
        build.arg("--release");
    }
    if cfg!(windows) {
        build.args(["-p", "gevrek", "-p", "gevrek-studio"]);
        build.args(["-p", "gevrek-shell"]);
    } else {
        build.args([
            "-p",
            "gevrek",
            "-p",
            "gevrek-smithay",
            "-p",
            "gevrek-studio",
        ]);
        build.args(["-p", "gevrek-shell", "-p", "gevrek-lock"]);
    }
    run("build", &mut build)?;

    let built = target_dir().join(if debug { "debug" } else { "release" });
    let bin = prefix.join("bin");
    let previous = prefix.join("lib").join("gevrek").join("previous");
    std::fs::create_dir_all(&bin)?;
    std::fs::create_dir_all(&previous)?;
    for name in programs() {
        let file = program_file(name);
        let from = built.join(&file);
        if !from.exists() {
            bail!("{} was not built", from.display());
        }
        let to = bin.join(&file);
        if to.exists() {
            // What ran until now stays available if the new build does not
            // come up.
            std::fs::copy(&to, previous.join(&file))?;
            // A program that is running cannot be written over, but it can
            // be replaced: the running one keeps its file.
            std::fs::remove_file(&to)?;
        }
        std::fs::copy(&from, &to).with_context(|| format!("copy to {}", to.display()))?;
        println!("installed {}", to.display());
    }
    remove_retired(&bin)?;
    println!(
        "The build installed before is in {}. Programs that are running keep running the old build until they are restarted.",
        previous.display()
    );
    if cfg!(windows) {
        keep_elevated_helper(&bin);
    } else {
        install_entries(&bin)?;
    }
    if arguments.iter().any(|argument| argument == "--session") {
        install_session(&prefix, &bin, &previous)?;
    }
    Ok(())
}

/// The task that starts the helper for administrator windows, of the
/// instance every install has (`gevrek_win32::elevated::helper_task_name`).
const ELEVATED_TASK: &str = "gevrek elevated helper (main)";

/// Windows: where the helper for the windows of administrator programs is
/// registered (`gevrek elevated install`, once), it is registered anew, as
/// the setup program and `install.ps1` do when they update.
///
/// The task starts a copy of the helper that is kept where only an
/// administrator can write, and that copy stayed the one of the build it
/// was registered with, whatever was installed since: it served until the
/// window manager and the helper no longer spoke the same, and then the
/// windows of administrator programs were left alone without a word but
/// one line in the log. Windows asks for an administrator each time.
/// Where the task is not registered nothing is done, and nothing is asked.
///
/// Whatever comes of it, the programs are installed: what went wrong is
/// said, and how to do it by hand.
fn keep_elevated_helper(bin: &Path) {
    let registered = Command::new("schtasks.exe")
        .args(["/query", "/tn", ELEVATED_TASK])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !registered {
        return;
    }
    println!(
        "The helper for administrator windows is registered: it is registered anew, for this build (Windows asks for an administrator)."
    );
    let gevrek = bin.join(program_file("gevrek"));
    let done = Command::new(&gevrek).args(["elevated", "install"]).status();
    if !done.is_ok_and(|status| status.success()) {
        println!(
            "The helper was not registered anew: the one kept from before is still the one that is started. A helper that runs holds its file; stop gevrek, then: {} elevated install",
            gevrek.display()
        );
    }
}

/// The programs a launcher lists (Linux): their entries, with the user's
/// other applications.
const ENTRIES: &[(&str, &str)] = &[("gevrek-studio.desktop", "gevrek-studio")];

/// Where the user's application entries are.
fn applications() -> Result<PathBuf> {
    Ok(std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(home()?.join(".local").join("share"))
        .join("applications"))
}

/// gevrek's icon as the icon theme every desktop falls back to has it
/// (`hicolor`): the files under `assets/brand/app-icon/linux/hicolor`, by
/// their place in the theme. Those of the release, not the nightly ones.
fn brand_icons() -> Vec<(PathBuf, PathBuf)> {
    let from = root()
        .join("assets")
        .join("brand")
        .join("app-icon")
        .join("linux")
        .join("hicolor");
    let mut found = Vec::new();
    for size in ["16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256", "512x512"] {
        let place = Path::new(size).join("apps").join("gevrek.png");
        found.push((from.join(&place), place));
    }
    let place = Path::new("scalable").join("apps").join("gevrek.svg");
    found.push((from.join(&place), place));
    found
}

/// Where the user's icons of that theme are.
fn icon_theme() -> Result<PathBuf> {
    Ok(applications()?
        .parent()
        .context("the folder of the user's data")?
        .join("icons")
        .join("hicolor"))
}

fn install_entries(bin: &Path) -> Result<()> {
    let applications = applications()?;
    std::fs::create_dir_all(&applications)?;
    // The icon the entries name (`Icon=gevrek`).
    let theme = icon_theme()?;
    for (from, place) in brand_icons() {
        let to = theme.join(place);
        if let Some(folder) = to.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::copy(&from, &to).with_context(|| format!("copy {}", from.display()))?;
    }
    println!("installed gevrek's icon in {}", theme.display());
    for (entry, program) in ENTRIES {
        // What starts it may not have the folder in its PATH: the full path.
        let text = std::fs::read_to_string(root().join("resources").join(entry))?.replace(
            &format!("Exec={program}"),
            &format!("Exec={}", bin.join(program).display()),
        );
        let to = applications.join(entry);
        // The entry of another install (another prefix) is left as it is.
        if to.exists() && !names(&to, bin) && names_a_program(&to) {
            println!(
                "kept {} (it starts the install in another folder)",
                to.display()
            );
            continue;
        }
        std::fs::write(&to, text)?;
        println!("installed {}", to.display());
    }
    Ok(())
}

/// Whether this file (a unit, an entry) names a program in `bin`: it then
/// belongs to the install in that prefix. The units and entries are in the
/// user's own folders whatever the prefix is, so this is what tells the
/// files of one install from those of another.
fn names(file: &Path, bin: &Path) -> bool {
    let folder = format!("{}{}", bin.display(), std::path::MAIN_SEPARATOR);
    std::fs::read_to_string(file).is_ok_and(|text| text.contains(&folder))
}

/// Whether the entry starts a program that is there.
fn names_a_program(entry: &Path) -> bool {
    std::fs::read_to_string(entry).is_ok_and(|text| {
        text.lines()
            .filter_map(|line| line.strip_prefix("Exec="))
            .filter_map(|command| command.split_whitespace().next())
            .any(|program| Path::new(program).is_absolute() && Path::new(program).exists())
    })
}

/// The files of a login session (Linux): the start script, the systemd
/// units of the user, and, prepared for root to put in place, the login
/// manager's entry and the portal selection.
fn install_session(prefix: &Path, bin: &Path, previous: &Path) -> Result<()> {
    if cfg!(windows) {
        bail!(
            "A login session is for Linux; on Windows gevrek starts with `gevrek enable-autostart`"
        );
    }
    let resources = root().join("resources");
    let fill = |text: String| {
        text.replace("@BIN@", &bin.to_string_lossy())
            .replace("@PREVIOUS@", &previous.to_string_lossy())
    };

    let script = bin.join("gevrek-session");
    std::fs::copy(resources.join("gevrek-session"), &script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("installed {}", script.display());

    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(home()?.join(".config"));
    let units = config_home.join("systemd").join("user");
    std::fs::create_dir_all(&units)?;
    for entry in std::fs::read_dir(resources.join("systemd"))? {
        let from = entry?.path();
        let to = units.join(from.file_name().context("a unit has a name")?);
        std::fs::write(&to, fill(std::fs::read_to_string(&from)?))?;
        println!("installed {}", to.display());
    }
    // (A unit an earlier install put there and this one no longer has:
    // the bar of before the shell.)
    for gone in RETIRED_UNITS {
        let stale = units.join(gone);
        if stale.exists() {
            std::fs::remove_file(&stale)?;
            println!("removed {} (no longer installed)", stale.display());
        }
    }

    // Which portal backend answers what, and gevrek's own backend: for
    // this user, where the portal looks before it looks at the system's
    // files, so that no root is needed for them. (Without the selection
    // the portal still answers, by whichever backend it finds last.)
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(home()?.join(".local").join("share"));
    for (name, folder) in [
        (
            "gevrek-portals.conf",
            config_home.join("xdg-desktop-portal"),
        ),
        (
            "gevrek.portal",
            data_home.join("xdg-desktop-portal").join("portals"),
        ),
    ] {
        std::fs::create_dir_all(&folder)?;
        let to = folder.join(name);
        std::fs::copy(resources.join(name), &to)?;
        println!("installed {}", to.display());
    }

    // What only root can put where it is looked for.
    let share = prefix.join("share").join("gevrek");
    std::fs::create_dir_all(&share)?;
    let entry = share.join("gevrek.desktop");
    // The login manager does not have the user's PATH: the full path.
    let text = std::fs::read_to_string(resources.join("gevrek.desktop"))?
        .replace("Exec=gevrek-session", &format!("Exec={}", script.display()));
    std::fs::write(&entry, text)?;
    let portals = share.join("gevrek-portals.conf");
    std::fs::copy(resources.join("gevrek-portals.conf"), &portals)?;
    // gevrek's own portal backend (gevrek-shell answers for it).
    let backend = share.join("gevrek.portal");
    std::fs::copy(resources.join("gevrek.portal"), &backend)?;
    // The user's units changed: the service manager reads them again.
    // (Not for an install into a staging folder, which is somebody else's.)
    let staged = std::env::var_os("XDG_CONFIG_HOME").is_some_and(|set| {
        std::env::var_os("HOME")
            .is_none_or(|home| Path::new(&set) != Path::new(&home).join(".config"))
    });
    let reloaded = !staged
        && Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .is_ok_and(|status| status.success());
    // What is for root to put in place, and only what is not there
    // already as it should be: after the first time there is nothing.
    let same = |ours: &Path, theirs: &str| {
        matches!(
            (std::fs::read(ours), std::fs::read(theirs)),
            (Ok(one), Ok(other)) if one == other
        )
    };
    let session_entry = "/usr/share/wayland-sessions/gevrek.desktop";
    let system_portals = "/usr/share/xdg-desktop-portal/gevrek-portals.conf";
    let system_backend = "/usr/share/xdg-desktop-portal/portals/gevrek.portal";
    println!();
    if !reloaded && !staged {
        println!("To finish: systemctl --user daemon-reload");
    }
    if same(&entry, session_entry) {
        println!("gevrek is in the login manager's list of sessions (nothing to do as root).");
    } else {
        println!("To finish, once (and again only if this file changes):");
        println!("  sudo install -Dm644 {} {session_entry}", entry.display());
        println!("Then gevrek is in the login manager's list of sessions.");
        println!("(Not needed where gevrek's own display manager logs you in.)");
    }
    if !same(&portals, system_portals) || !same(&backend, system_backend) {
        println!("(For every user of the machine, not only this one, also:");
        println!(
            "  sudo install -Dm644 {} {system_portals}",
            portals.display()
        );
        println!(
            "  sudo install -Dm644 {} {system_backend} )",
            backend.display()
        );
    }
    println!(
        "  --safe:     {} --safe      (ignores the configuration file)",
        script.display()
    );
    println!(
        "  --previous: {} --previous  (the build installed before)",
        script.display()
    );
    println!("Ctrl+Alt+Backspace twice ends a session whatever its configuration says.");
    Ok(())
}

fn uninstall(arguments: &[String]) -> Result<()> {
    let prefix = prefix(arguments)?;
    let bin = prefix.join("bin");
    remove_retired(&bin)?;
    for name in programs() {
        let file = bin.join(program_file(name));
        if file.exists() {
            std::fs::remove_file(&file)?;
            println!("removed {}", file.display());
        }
    }
    for kept in [
        prefix.join("lib").join("gevrek"),
        prefix.join("share").join("gevrek"),
    ] {
        if kept.exists() {
            std::fs::remove_dir_all(&kept)?;
            println!("removed {}", kept.display());
        }
    }
    let script = prefix.join("bin").join("gevrek-session");
    if script.exists() {
        std::fs::remove_file(&script)?;
        println!("removed {}", script.display());
    }
    if let Ok(applications) = applications() {
        // The icon goes when no entry of another install is left to name it.
        let mut another = false;
        for (entry, _) in ENTRIES {
            let file = applications.join(entry);
            if !file.exists() {
                continue;
            }
            if names(&file, &bin) {
                std::fs::remove_file(&file)?;
                println!("removed {}", file.display());
            } else {
                another = true;
                println!("kept {} (not of this install)", file.display());
            }
        }
        if let (false, Ok(theme)) = (another, icon_theme()) {
            for (_, place) in brand_icons() {
                let file = theme.join(place);
                if file.exists() {
                    std::fs::remove_file(&file)?;
                }
            }
        }
    }
    if let Ok(home) = home() {
        let units = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or(home.join(".config"))
            .join("systemd")
            .join("user");
        // The units of a session are in the user's folder whatever the
        // prefix: they are taken away only when they start the programs of
        // this prefix. Uninstalling a trial in another folder leaves the
        // session the user logs into alone.
        let installed: Vec<PathBuf> = std::fs::read_dir(root().join("resources").join("systemd"))
            .map(|resources| {
                resources
                    .flatten()
                    .map(|entry| units.join(entry.file_name()))
                    .filter(|unit| unit.exists())
                    .collect()
            })
            .unwrap_or_default();
        if installed.iter().any(|unit| names(unit, &bin)) {
            for unit in installed {
                std::fs::remove_file(&unit)?;
                println!("removed {}", unit.display());
            }
            // The portal files of that session go with it.
            let config_home = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or(home.join(".config"));
            let data_home = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or(home.join(".local").join("share"));
            for file in [
                config_home
                    .join("xdg-desktop-portal")
                    .join("gevrek-portals.conf"),
                data_home
                    .join("xdg-desktop-portal")
                    .join("portals")
                    .join("gevrek.portal"),
            ] {
                if file.exists() {
                    std::fs::remove_file(&file)?;
                    println!("removed {}", file.display());
                }
            }
        } else if !installed.is_empty() {
            println!(
                "kept the session's units in {} (they start the install in another folder)",
                units.display()
            );
        }
    }
    println!("If they were installed: sudo rm /usr/share/wayland-sessions/gevrek.desktop /usr/share/xdg-desktop-portal/gevrek-portals.conf");
    Ok(())
}

/// Build the programs and make an RPM of them (Fedora and its relatives):
/// `target/rpm/RPMS/<arch>/gevrek-<version>-<n>.<dist>.<arch>.rpm`.
/// `--debug` packages the debug build (faster to make, larger, slower).
fn rpm(arguments: &[String]) -> Result<()> {
    if cfg!(windows) {
        bail!("an RPM is made on Linux");
    }
    let debug = arguments.iter().any(|argument| argument == "--debug");
    let programs = build_linux_programs(debug)?;
    // The commits since the start count the releases of one version.
    let release = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(root())
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && text.chars().all(|letter| letter.is_ascii_digit()))
        .unwrap_or_else(|| "1".to_string());
    let top = rpm_build(&programs, &release)?;
    println!();
    println!("The package is under {}", top.join("RPMS").display());
    println!(
        "  sudo dnf install <the file>     then gevrek is in the login manager's list of sessions"
    );
    Ok(())
}

/// Build the programs of a Linux package; returns the folder they are in.
fn build_linux_programs(debug: bool) -> Result<PathBuf> {
    let mut build = cargo();
    build.arg("build").arg("--bins");
    if !debug {
        build.arg("--release");
    }
    build.args([
        "-p",
        "gevrek",
        "-p",
        "gevrek-smithay",
        "-p",
        "gevrek-studio",
        "-p",
        "gevrek-shell",
        "-p",
        "gevrek-lock",
    ]);
    run("build", &mut build)?;
    Ok(target_dir().join(if debug { "debug" } else { "release" }))
}

/// The version as a package manager sorts it: a pre-release (`0.7.0-rc.1`)
/// before its release, which `~` says to dpkg and to rpm.
fn package_version() -> String {
    env!("CARGO_PKG_VERSION").replacen('-', "~", 1)
}

/// Make the RPM of programs that are built (`packaging/rpm/gevrek.spec`);
/// returns rpmbuild's folder (`target/rpm`), with the package under `RPMS`.
fn rpm_build(programs: &Path, release: &str) -> Result<PathBuf> {
    let top = target_dir().join("rpm");
    // Only the package of this build is there afterwards.
    if top.join("RPMS").exists() {
        std::fs::remove_dir_all(top.join("RPMS"))?;
    }
    std::fs::create_dir_all(&top)?;
    let version = package_version();
    let version = version.as_str();
    let define = |name: &str, value: &str| format!("{name} {value}");
    run(
        "rpmbuild",
        Command::new("rpmbuild")
            .arg("-bb")
            .arg("--define")
            .arg(define("_topdir", &top.to_string_lossy()))
            .arg("--define")
            .arg(define("gevrek_version", version))
            .arg("--define")
            .arg(define("gevrek_release", release))
            .arg("--define")
            .arg(define("gevrek_programs", &programs.to_string_lossy()))
            .arg("--define")
            .arg(define(
                "gevrek_resources",
                &root().join("resources").to_string_lossy(),
            ))
            .arg("--define")
            .arg(define("gevrek_root", &root().to_string_lossy()))
            .arg(root().join("packaging").join("rpm").join("gevrek.spec")),
    )?;
    Ok(top)
}

/// The programs and `packaging/windows` in one folder and one zip under
/// `target/package`, named by the version and the commit they are of.
fn package(arguments: &[String]) -> Result<()> {
    if !cfg!(windows) {
        bail!("`package` makes the Windows package; on Linux there is `cargo xtask rpm`");
    }
    let debug = arguments.iter().any(|argument| argument == "--debug");
    // Which commit: asked of git, which only reads.
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let name = format!("gevrek-{}-{commit}-windows-x64", env!("CARGO_PKG_VERSION"));
    let (folder, zip) = windows_package(&target_dir().join("package"), &name, debug)?;
    println!("== the package is {}", zip.display());
    println!("   (unpacked: {}; install.ps1 in it installs or updates)", folder.display());
    Ok(())
}

/// Build the Windows programs and put them, with the scripts that install
/// and remove them, in the folder `<out>/<name>` and the zip `<out>/<name>.zip`
/// (which holds that one folder).
fn windows_package(out: &Path, name: &str, debug: bool) -> Result<(PathBuf, PathBuf)> {
    let mut build = cargo();
    build.arg("build").arg("--bins");
    if !debug {
        build.arg("--release");
    }
    build.args(["-p", "gevrek", "-p", "gevrek-studio", "-p", "gevrek-shell"]);
    run("build", &mut build)?;
    let built = target_dir().join(if debug { "debug" } else { "release" });
    let folder = out.join(name);
    if folder.exists() {
        std::fs::remove_dir_all(&folder)?;
    }
    std::fs::create_dir_all(&folder)?;
    for program in programs() {
        let file = program_file(program);
        let from = built.join(&file);
        if !from.exists() {
            bail!("{} was not built", from.display());
        }
        std::fs::copy(&from, folder.join(&file))?;
    }
    let scripts = root().join("packaging").join("windows");
    for file in ["install.ps1", "uninstall.ps1", "README.txt"] {
        std::fs::copy(scripts.join(file), folder.join(file))
            .with_context(|| format!("packaging/windows/{file}"))?;
    }
    for file in ["LICENSE", "LICENSE.md", "LICENSE.txt"] {
        if root().join(file).exists() {
            std::fs::copy(root().join(file), folder.join(file))?;
        }
    }
    // The zip: by the `tar` that is part of Windows (it writes a zip when
    // the name ends so).
    let zip = out.join(format!("{name}.zip"));
    if zip.exists() {
        std::fs::remove_file(&zip)?;
    }
    run(
        "zip",
        Command::new("tar")
            .arg("-a")
            .arg("-c")
            .arg("-f")
            .arg(&zip)
            .arg("-C")
            .arg(out)
            .arg(name),
    )?;
    Ok((folder, zip))
}

/// The folder of a tool that is not on the PATH of every machine.
fn find_tool(name: &str, places: &[PathBuf]) -> Option<PathBuf> {
    let on_path = Command::new(name)
        .arg("/?")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    if on_path {
        return Some(PathBuf::from(name));
    }
    places.iter().map(|place| place.join(name)).find(|file| file.exists())
}

fn has_program(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// What a release offers, under `target/dist`, by names that carry no
/// version (the address of the newest one is then always the same, and it
/// is by these names that an installed gevrek finds its update:
/// `crates/update`):
///
/// - Windows: `gevrek-windows-x64.zip` (the programs with `install.ps1`)
///   and `gevrek-windows-x64-setup.exe` (Inno Setup 6,
///   `packaging/windows/gevrek.iss`);
/// - Linux: `gevrek-linux-x64.tar.gz` (the files as a package places
///   them), `gevrek-linux-x64.deb` (dpkg) and `gevrek-linux-x64.rpm`
///   (rpmbuild, `packaging/rpm/gevrek.spec`).
///
/// `cargo xtask dist` makes all of them that this machine has the tools
/// for and says which it left out; `cargo xtask dist deb rpm` makes the
/// named ones and fails if one cannot be made.
fn dist(arguments: &[String]) -> Result<()> {
    let debug = arguments.iter().any(|argument| argument == "--debug");
    let named: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| !argument.starts_with("--"))
        .collect();
    let kinds: &[&str] = if cfg!(windows) {
        &["zip", "setup"]
    } else {
        &["tar", "deb", "rpm"]
    };
    if let Some(unknown) = named.iter().find(|kind| !kinds.contains(kind)) {
        bail!("dist {unknown}: on this system there are {}", kinds.join(", "));
    }
    let wanted = |kind: &str| named.is_empty() || named.contains(&kind);
    // Left out, with a word, when nobody named it; an error when somebody did.
    let without = |kind: &str, tool: &str| -> Result<()> {
        if named.contains(&kind) {
            bail!("{tool} is not installed: no {kind}");
        }
        println!("== {kind}: left out ({tool} is not installed)");
        Ok(())
    };
    let (label, deb_arch) = match std::env::consts::ARCH {
        "x86_64" => ("x64", "amd64"),
        "aarch64" => ("arm64", "arm64"),
        other => bail!("no package is made for {other}"),
    };
    let out = target_dir().join("dist");
    std::fs::create_dir_all(&out)?;
    let mut made: Vec<PathBuf> = Vec::new();

    if cfg!(windows) {
        let name = format!("gevrek-windows-{label}");
        let (folder, zip) = windows_package(&out, &name, debug)?;
        if wanted("zip") {
            made.push(zip);
        }
        if wanted("setup") {
            let places: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
                .iter()
                .filter_map(std::env::var_os)
                .map(|programs| PathBuf::from(programs).join("Inno Setup 6"))
                .collect();
            match find_tool("ISCC.exe", &places) {
                Some(compiler) => {
                    // (The numbers alone: what Windows shows as the file's version.)
                    let numbers = env!("CARGO_PKG_VERSION")
                        .split(['-', '+'])
                        .next()
                        .unwrap_or_default();
                    run(
                        "setup program (Inno Setup)",
                        Command::new(compiler)
                            .arg("/Qp")
                            .arg(format!("/DVersion={}", env!("CARGO_PKG_VERSION")))
                            .arg(format!("/DNumbers={numbers}"))
                            .arg(format!("/DSource={}", folder.display()))
                            .arg(format!("/DOut={}", out.display()))
                            .arg(format!("/DName={name}-setup"))
                            .arg(root().join("packaging").join("windows").join("gevrek.iss")),
                    )?;
                    made.push(out.join(format!("{name}-setup.exe")));
                }
                None => without("setup", "Inno Setup 6 (ISCC.exe)")?,
            }
        }
    } else {
        let name = format!("gevrek-linux-{label}");
        let programs = build_linux_programs(debug)?;
        let work = out.join("work");
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        if wanted("tar") {
            let folder = work.join(&name);
            linux_tree(&programs, &folder, PAM_FEDORA)?;
            std::fs::copy(root().join("LICENSE"), folder.join("LICENSE"))?;
            std::fs::copy(
                root().join("packaging").join("linux").join("README.txt"),
                folder.join("README.txt"),
            )?;
            mode(&folder.join("LICENSE"), false)?;
            mode(&folder.join("README.txt"), false)?;
            let file = out.join(format!("{name}.tar.gz"));
            run(
                "tar",
                Command::new("tar")
                    .args(["--owner=0", "--group=0", "-czf"])
                    .arg(&file)
                    .arg("-C")
                    .arg(&work)
                    .arg(&name),
            )?;
            made.push(file);
        }
        if wanted("deb") {
            if has_program("dpkg-deb") && has_program("dpkg-shlibdeps") {
                made.push(deb(&programs, &work, &out, &name, deb_arch)?);
            } else {
                without("deb", "dpkg-deb with dpkg-shlibdeps (the package dpkg-dev)")?;
            }
        }
        if wanted("rpm") {
            if has_program("rpmbuild") {
                let top = rpm_build(&programs, "1")?;
                let mut found = Vec::new();
                for arch in std::fs::read_dir(top.join("RPMS"))? {
                    for file in std::fs::read_dir(arch?.path())? {
                        found.push(file?.path());
                    }
                }
                let [package] = found.as_slice() else {
                    bail!("rpmbuild left {} packages, not one", found.len());
                };
                let file = out.join(format!("{name}.rpm"));
                std::fs::copy(package, &file)?;
                made.push(file);
            } else {
                without("rpm", "rpmbuild")?;
            }
        }
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
    }
    println!("== made:");
    for file in &made {
        println!("   {}", file.display());
    }
    Ok(())
}

/// How the lock screen checks the password, as every login on the system
/// does: Fedora and its relatives, and Debian and its.
const PAM_FEDORA: &str = "#%PAM-1.0\nauth       include      system-auth\naccount    include      system-auth\n";
const PAM_DEBIAN: &str = "#%PAM-1.0\n@include common-auth\n@include common-account\n";

/// The files of gevrek as a package places them on a Linux system, under
/// `tree` (`usr/bin`, `usr/lib/systemd/user`, `usr/share`, `etc/pam.d`):
/// what `packaging/rpm/gevrek.spec` installs, for the packages that are
/// not made by rpmbuild.
fn linux_tree(built: &Path, tree: &Path, pam: &str) -> Result<()> {
    let put = |from: &Path, to: &str, runs: bool| -> Result<()> {
        let to = tree.join(to);
        std::fs::create_dir_all(to.parent().context("a file is in a folder")?)?;
        std::fs::copy(from, &to).with_context(|| format!("copy {}", from.display()))?;
        mode(&to, runs)
    };
    let resources = root().join("resources");
    for program in programs() {
        put(&built.join(program), &format!("usr/bin/{program}"), true)?;
    }
    put(&resources.join("gevrek-session"), "usr/bin/gevrek-session", true)?;
    // The units name the programs by their place.
    for unit in [
        "gevrek.service",
        "gevrek-safe.service",
        "gevrek-shell.service",
        "gevrek-shutdown.target",
    ] {
        let to = tree.join("usr/lib/systemd/user").join(unit);
        std::fs::create_dir_all(to.parent().context("a file is in a folder")?)?;
        let text = std::fs::read_to_string(resources.join("systemd").join(unit))?
            .replace("@BIN@", "/usr/bin");
        std::fs::write(&to, text)?;
        mode(&to, false)?;
    }
    put(
        &resources.join("gevrek.desktop"),
        "usr/share/wayland-sessions/gevrek.desktop",
        false,
    )?;
    put(
        &resources.join("gevrek-studio.desktop"),
        "usr/share/applications/gevrek-studio.desktop",
        false,
    )?;
    for (from, place) in brand_icons() {
        put(
            &from,
            &format!("usr/share/icons/hicolor/{}", place.to_string_lossy()),
            false,
        )?;
    }
    put(
        &resources.join("gevrek-portals.conf"),
        "usr/share/xdg-desktop-portal/gevrek-portals.conf",
        false,
    )?;
    put(
        &resources.join("gevrek.portal"),
        "usr/share/xdg-desktop-portal/portals/gevrek.portal",
        false,
    )?;
    put(&root().join("docs").join("LINUX.md"), "usr/share/doc/gevrek/LINUX.md", false)?;
    put(&root().join("LICENSE"), "usr/share/licenses/gevrek/LICENSE", false)?;
    let lock = tree.join("etc/pam.d/gevrek-lock");
    std::fs::create_dir_all(lock.parent().context("a file is in a folder")?)?;
    std::fs::write(&lock, pam)?;
    mode(&lock, false)
}

/// A program may be run by everybody, another file read by everybody.
fn mode(file: &Path, runs: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if runs { 0o755 } else { 0o644 };
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(mode))?;
    }
    let _ = (file, runs);
    Ok(())
}

/// How much room a folder takes, in units of 1024 bytes (what a .deb says
/// of itself).
fn size_in_kib(folder: &Path) -> u64 {
    let mut bytes = 0;
    let mut left = vec![folder.to_path_buf()];
    while let Some(folder) = left.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            match entry.metadata() {
                Ok(told) if told.is_dir() => left.push(entry.path()),
                Ok(told) => bytes += told.len(),
                Err(_) => {}
            }
        }
    }
    bytes.div_ceil(1024)
}

/// The .deb: the tree, and what dpkg is to know of it
/// (`packaging/deb/control`). The libraries it depends on are asked of
/// dpkg-shlibdeps, which reads them from the programs: they are those of
/// the system the programs were built on.
fn deb(programs: &Path, work: &Path, out: &Path, name: &str, arch: &str) -> Result<PathBuf> {
    let tree = work.join("deb");
    linux_tree(programs, &tree, PAM_DEBIAN)?;
    std::fs::copy(
        root().join("LICENSE"),
        tree.join("usr/share/doc/gevrek/copyright"),
    )?;
    // dpkg-shlibdeps wants to be in a source package: a `debian/control`
    // is enough of one.
    let asked = work.join("shlibs");
    std::fs::create_dir_all(asked.join("debian"))?;
    std::fs::write(
        asked.join("debian/control"),
        format!("Source: gevrek\n\nPackage: gevrek\nArchitecture: {arch}\n"),
    )?;
    let mut ask = Command::new("dpkg-shlibdeps");
    ask.arg("-O").current_dir(&asked);
    for program in self::programs() {
        ask.arg(tree.join("usr/bin").join(program));
    }
    let told = ask.output().context("dpkg-shlibdeps")?;
    let libraries = String::from_utf8_lossy(&told.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends=").map(str::to_string))
        .filter(|libraries| told.status.success() && !libraries.trim().is_empty())
        .with_context(|| {
            format!(
                "dpkg-shlibdeps did not say what the programs need: {}",
                String::from_utf8_lossy(&told.stderr).trim()
            )
        })?;
    let control = std::fs::read_to_string(root().join("packaging").join("deb").join("control"))?
        .replace("@VERSION@", &package_version())
        .replace("@ARCH@", arch)
        .replace("@SIZE@", &size_in_kib(&tree).to_string())
        .replace("@LIBRARIES@", libraries.trim());
    std::fs::create_dir_all(tree.join("DEBIAN"))?;
    std::fs::write(tree.join("DEBIAN/control"), control)?;
    // Kept as the person changed it when the package is updated.
    std::fs::write(tree.join("DEBIAN/conffiles"), "/etc/pam.d/gevrek-lock\n")?;
    let file = out.join(format!("{name}.deb"));
    run(
        "dpkg-deb",
        Command::new("dpkg-deb")
            .args(["--root-owner-group", "--build"])
            .arg(&tree)
            .arg(&file),
    )?;
    Ok(file)
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("check") => check(),
        Some("package") => package(&arguments[1..]),
        Some("install") => install(&arguments[1..]),
        Some("uninstall") => uninstall(&arguments[1..]),
        Some("rpm") => rpm(&arguments[1..]),
        Some("dist") => dist(&arguments[1..]),
        _ => {
            println!("cargo xtask check | install [--prefix <dir>] [--debug] | uninstall [--prefix <dir>] | rpm [--debug] | package [--debug] | dist [zip|setup|tar|deb|rpm ...] [--debug]");
            Ok(())
        }
    }
}
