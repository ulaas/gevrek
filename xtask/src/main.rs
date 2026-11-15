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
//!   `<prefix>/lib/gevrek/previous`. With `--session` (Linux) the files of
//!   a login session are installed too (`resources/README.md`), and the
//!   two commands that need root are printed.
//! - `uninstall [--prefix <dir>]`: remove what `install` put there.
//! - `package [--debug]` (Windows): the programs with the scripts that
//!   install, update and remove them for one person, in one zip
//!   (`target/package`): what is given to a machine without the sources.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The programs of this system.
fn programs() -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
            "gevrek",
            "gevrek-wm",
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
                    .env("CARGO_TARGET_DIR", root().join("target").join("win")),
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

    let built = root()
        .join("target")
        .join(if debug { "debug" } else { "release" });
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
    if !cfg!(windows) {
        install_entries(&bin)?;
    }
    if arguments.iter().any(|argument| argument == "--session") {
        install_session(&prefix, &bin, &previous)?;
    }
    Ok(())
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
    for gone in ["gevrek-bar.service"] {
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
    let programs = root()
        .join("target")
        .join(if debug { "debug" } else { "release" });
    let top = root().join("target").join("rpm");
    std::fs::create_dir_all(&top)?;
    let version = env!("CARGO_PKG_VERSION");
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
            .arg(define("gevrek_release", &release))
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
    println!();
    println!("The package is under {}", top.join("RPMS").display());
    println!(
        "  sudo dnf install <the file>     then gevrek is in the login manager's list of sessions"
    );
    Ok(())
}

/// The programs and `packaging/windows` in one folder and one zip under
/// `target/package`, named by the version and the commit they are of.
fn package(arguments: &[String]) -> Result<()> {
    if !cfg!(windows) {
        bail!("`package` makes the Windows package; on Linux there is `cargo xtask rpm`");
    }
    let debug = arguments.iter().any(|argument| argument == "--debug");
    let mut build = cargo();
    build.arg("build").arg("--bins");
    if !debug {
        build.arg("--release");
    }
    build.args(["-p", "gevrek", "-p", "gevrek-studio", "-p", "gevrek-shell"]);
    run("build", &mut build)?;
    let built = root()
        .join("target")
        .join(if debug { "debug" } else { "release" });
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
    let out = root().join("target").join("package");
    let folder = out.join(&name);
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
            .arg(&out)
            .arg(&name),
    )?;
    println!("== the package is {}", zip.display());
    println!("   (unpacked: {}; install.ps1 in it installs or updates)", folder.display());
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("check") => check(),
        Some("package") => package(&arguments[1..]),
        Some("install") => install(&arguments[1..]),
        Some("uninstall") => uninstall(&arguments[1..]),
        Some("rpm") => rpm(&arguments[1..]),
        _ => {
            println!("cargo xtask check | install [--prefix <dir>] [--debug] | uninstall [--prefix <dir>] | rpm [--debug] | package [--debug]");
            Ok(())
        }
    }
}
