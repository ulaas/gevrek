//! gevrek's update: which release is the newest, and (on Windows) putting
//! it in place of the one that is installed.
//!
//! GitHub is asked for the latest release of `ulaas/gevrek`. The request
//! carries nothing but a `gevrek/<version>` User-Agent. The package of a
//! newer release is downloaded from github.com over HTTPS and used only if
//! its SHA-256 is the one the release's `SHA256SUMS.txt` names. (That list
//! comes from the same place as the package: it catches a download that
//! went wrong, not a release somebody else made.)
//!
//! An update has two steps, so that a desktop in use is not taken away
//! under its windows:
//!
//! 1. [`stage`]: download, check and unpack, into gevrek's state folder.
//!    Nothing that runs is touched.
//! 2. [`apply`]: move the installed programs aside (to `previous`) and put
//!    the staged ones in their place. `gevrek start` does this before it
//!    starts anything, `gevrek update` does it at once.
//!
//! Only an install made from the package is updated (see [`is_packaged`]):
//! a build somebody made from the sources is theirs, and on Linux the
//! package manager is the one that updates.
//!
//! The network and the archive are left to two programs that are part of
//! Windows, `curl.exe` and `tar.exe`, as `schtasks` is for the sign-in
//! task: no TLS and no zip code of gevrek's own.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a person reads about the newest release and gets it by hand.
pub const RELEASES_PAGE: &str = "https://github.com/ulaas/gevrek/releases/latest";
/// The package an installed gevrek on Windows is updated from: a zip that
/// holds one folder of the same name (`cargo xtask dist` makes it).
pub const WINDOWS_PACKAGE: &str = "gevrek-windows-x64";
const SUMS: &str = "SHA256SUMS.txt";
/// The programs of the Windows package.
pub const WINDOWS_PROGRAMS: &[&str] = &[
    "gevrek.exe",
    "gevrek-wm.exe",
    "gevrek-studio.exe",
    "gevrek-shell.exe",
];
/// Programs a package has since a later version than the first: put in
/// place where the package has them, and not asked for of an install
/// (one made before they were is still one to update).
const LATER_PROGRAMS: &[&str] = &["gevrek-elevated.exe"];
/// What else of the package is put beside the programs when it is there.
const BESIDE: &[&str] = &["uninstall.ps1", "README.txt", "LICENSE"];
/// Seconds between two checks that nobody asked for.
const CHECK_EVERY: u64 = 86_400;
/// No package is anywhere near this size; a larger one is refused.
const MAX_PACKAGE: u64 = 512 << 20;
const MAX_TEXT: u64 = 1 << 20;

/// The version of this build.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------
// Versions

/// The numbers of `1.2.3`, and whether a pre-release follows (`1.2.3-rc.1`).
fn parse_version(text: &str) -> Option<(Vec<u64>, bool)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let text = text.split('+').next()?;
    let (core, pre) = match text.split_once('-') {
        Some((core, _)) => (core, true),
        None => (text, false),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    (!numbers.is_empty() && numbers.len() <= 4).then_some((numbers, pre))
}

/// Whether `candidate` is a later version than `current`, number by number
/// (0.10.0 is later than 0.9.3). A release is later than its own
/// pre-releases. What is no version is never later.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some((mut one, one_pre)), Some((mut other, other_pre))) =
        (parse_version(candidate), parse_version(current))
    else {
        return false;
    };
    let length = one.len().max(other.len());
    one.resize(length, 0);
    other.resize(length, 0);
    match one.cmp(&other) {
        std::cmp::Ordering::Equal => other_pre && !one_pre,
        order => order == std::cmp::Ordering::Greater,
    }
}

// ---------------------------------------------------------------------
// What is remembered between runs

/// `update.json` in the update folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether gevrek looks for a newer release by itself (once a day, when
    /// it starts) and installs it the next time it starts.
    pub automatic: bool,
    /// When GitHub was last asked, in seconds since 1970.
    pub last_check: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            automatic: true,
            last_check: None,
        }
    }
}

impl Settings {
    pub fn load(folder: &Path) -> Self {
        let file = folder.join("update.json");
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                log::debug!(
                    "the update settings are the defaults: {} was not read: {error}",
                    file.display()
                );
                return Self::default();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            log::debug!(
                "the update settings are the defaults: {} is not read: {error}",
                file.display()
            );
            Self::default()
        })
    }

    pub fn save(&self, folder: &Path) -> Result<()> {
        std::fs::create_dir_all(folder)?;
        let file = folder.join("update.json");
        std::fs::write(&file, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("write {}", file.display()))?;
        log::debug!("the update settings were written to {}", file.display());
        Ok(())
    }

    /// Whether a check nobody asked for is due: at most once a day, and
    /// again if the clock was set back past the last one.
    pub fn check_due(&self, now: u64) -> bool {
        self.automatic
            && self
                .last_check
                .is_none_or(|last| last > now || now - last >= CHECK_EVERY)
    }
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

// ---------------------------------------------------------------------
// Releases

/// Where releases come from: GitHub, outside this crate's own tests.
#[derive(Clone, Debug)]
pub struct Source {
    /// The address that answers with the latest release as JSON.
    api: String,
    /// Every file of a release has an address that starts with this.
    downloads: String,
    /// Nothing but HTTPS, redirects included.
    https_only: bool,
}

impl Source {
    pub fn github() -> Self {
        Self {
            api: "https://api.github.com/repos/ulaas/gevrek/releases/latest".to_string(),
            downloads: "https://github.com/ulaas/gevrek/releases/download/".to_string(),
            https_only: true,
        }
    }
}

/// A file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// The newest release, as far as an update needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Without the `v` of the tag.
    pub version: String,
    /// Its page, for a person.
    pub page: String,
    /// The Windows package and the list of checksums, when the release has
    /// them.
    pub package: Option<Asset>,
    pub sums: Option<Asset>,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
}

/// What GitHub's answer says. A file whose address is not one of this
/// source's downloads is not taken.
fn parse_release(text: &str, source: &Source) -> Result<Release> {
    let told: ReleaseJson =
        serde_json::from_str(text).context("the answer about the latest release is not one")?;
    let version = told.tag_name.trim_start_matches(['v', 'V']).to_string();
    if parse_version(&version).is_none() {
        bail!(
            "the latest release is named {}, which is no version",
            told.tag_name
        );
    }
    let find = |name: &str| {
        told.assets
            .iter()
            .find(|asset| {
                asset.name == name && asset.browser_download_url.starts_with(&source.downloads)
            })
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
            })
    };
    Ok(Release {
        package: find(&format!("{WINDOWS_PACKAGE}.zip")),
        sums: find(SUMS),
        page: if told.html_url.starts_with("https://github.com/") {
            told.html_url
        } else {
            RELEASES_PAGE.to_string()
        },
        version,
    })
}

/// A program that is part of Windows, by its place there (not by the PATH,
/// which is the person's). Elsewhere by its name.
fn system_program(name: &str) -> PathBuf {
    if cfg!(windows) {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let file = Path::new(&root)
                .join("System32")
                .join(format!("{name}.exe"));
            if file.exists() {
                return file;
            }
        }
    }
    PathBuf::from(name)
}

fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.stdin(Stdio::null())
}

/// Fetch one address into a file.
fn download(source: &Source, url: &str, to: &Path, most: u64, seconds: u32) -> Result<()> {
    let mut curl = Command::new(system_program("curl"));
    curl.args(["--silent", "--show-error", "--fail", "--location"])
        .args(["--connect-timeout", "20"])
        .args(["--max-time", &seconds.to_string()])
        .args(["--max-filesize", &most.to_string()])
        .args(["--user-agent", &format!("gevrek/{}", current())])
        .args(["--header", "Accept: application/vnd.github+json, */*"]);
    if source.https_only {
        curl.args(["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"]);
    }
    curl.arg("--output").arg(to).arg(url);
    log::debug!("{url} is downloaded to {}", to.display());
    let output = quiet(&mut curl)
        .output()
        .context("curl could not be started (it is part of Windows 10 and later)")?;
    if !output.status.success() {
        let _ = std::fs::remove_file(to);
        let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
        log::debug!(
            "{url} was not downloaded: curl ended with {}",
            output.status
        );
        bail!(
            "{url}: {}",
            if said.is_empty() {
                "the download failed"
            } else {
                &said
            }
        );
    }
    let size = std::fs::metadata(to)?.len();
    if size > most {
        let _ = std::fs::remove_file(to);
        bail!("{url}: {size} bytes is more than a file of a release has");
    }
    log::debug!("{url} was downloaded: {size} bytes");
    Ok(())
}

/// Ask for the latest release. `folder` is the update folder (the answer
/// passes through a file there).
pub fn latest(source: &Source, folder: &Path) -> Result<Release> {
    std::fs::create_dir_all(folder)?;
    let file = folder.join("latest.json");
    log::debug!("the latest release is asked for at {}", source.api);
    download(source, &source.api, &file, MAX_TEXT, 60)?;
    let text = std::fs::read_to_string(&file);
    let _ = std::fs::remove_file(&file);
    let release = parse_release(&text?, source).inspect_err(|error| {
        log::debug!("the answer about the latest release is not read: {error:#}")
    })?;
    log::debug!(
        "the latest release is {} (this is {}): {}, {}",
        release.version,
        current(),
        if release.package.is_some() {
            "with a Windows package"
        } else {
            "without a Windows package"
        },
        if release.sums.is_some() {
            "with checksums"
        } else {
            "without checksums"
        }
    );
    Ok(release)
}

// ---------------------------------------------------------------------
// Staging

/// The checksum `SHA256SUMS.txt` names for a file (the lines `sha256sum`
/// writes: the sum, then the name, which may have a `*` before it).
fn sum_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| sum.to_ascii_lowercase())
    })
}

fn sha256_of(file: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut opened = std::fs::File::open(file)?;
    std::io::copy(&mut opened, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn staged_folder(folder: &Path) -> PathBuf {
    folder.join("staged")
}

/// The update that is staged and complete: its version and the folder of
/// its programs.
pub fn staged(folder: &Path) -> Option<(String, PathBuf)> {
    let staged = staged_folder(folder);
    let version = std::fs::read_to_string(staged.join("ready")).ok()?;
    let version = version.trim().to_string();
    parse_version(&version)?;
    let package = staged.join(WINDOWS_PACKAGE);
    WINDOWS_PROGRAMS
        .iter()
        .all(|program| package.join(program).is_file())
        .then_some((version, package))
}

/// Take away what is staged (it was applied, or is of no use any more).
pub fn unstage(folder: &Path) {
    let staged = staged_folder(folder);
    match std::fs::remove_dir_all(&staged) {
        Ok(()) => log::debug!("what was staged in {} was taken away", staged.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::debug!("{} was not taken away: {error}", staged.display()),
    }
}

/// Download the release's Windows package, check it, and unpack it in the
/// update folder. Returns the folder of its programs. What was staged
/// before is replaced.
pub fn stage(source: &Source, release: &Release, folder: &Path) -> Result<PathBuf> {
    let package = release
        .package
        .as_ref()
        .ok_or_else(|| anyhow!("release {} has no {WINDOWS_PACKAGE}.zip", release.version))?;
    let sums = release
        .sums
        .as_ref()
        .ok_or_else(|| anyhow!("release {} has no {SUMS}", release.version))?;
    let work = folder.join("download");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    // Whether what was staged before is gone already: until then a
    // failure (no network, a file that is not the release's) leaves it as
    // it was, ready to be put in place.
    let mut replaced = false;
    log::debug!(
        "release {} is staged in {}",
        release.version,
        folder.display()
    );
    let result = (|| {
        let sums_file = work.join(SUMS);
        download(source, &sums.url, &sums_file, MAX_TEXT, 60)?;
        let expected =
            sum_of(&std::fs::read_to_string(&sums_file)?, &package.name).ok_or_else(|| {
                anyhow!(
                    "{SUMS} of release {} does not name {}",
                    release.version,
                    package.name
                )
            })?;
        let zip = work.join(&package.name);
        download(source, &package.url, &zip, MAX_PACKAGE, 1800)?;
        let found = sha256_of(&zip)?;
        if found != expected {
            bail!(
                "{} is not the file the release names (SHA-256 {found}, expected {expected}); nothing was changed",
                package.name
            );
        }
        log::debug!(
            "{} is the file the release names (SHA-256 {found})",
            package.name
        );
        replaced = true;
        unstage(folder);
        let staged = staged_folder(folder);
        std::fs::create_dir_all(&staged)?;
        let mut tar = Command::new(system_program("tar"));
        tar.arg("-xf").arg(&zip).arg("-C").arg(&staged);
        let output = quiet(&mut tar)
            .output()
            .context("tar could not be started")?;
        if !output.status.success() {
            bail!(
                "{} could not be unpacked: {}",
                package.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let programs = staged.join(WINDOWS_PACKAGE);
        for program in WINDOWS_PROGRAMS {
            if !programs.join(program).is_file() {
                bail!("{} has no {program}", package.name);
            }
        }
        // Written last: what is staged counts only with this.
        std::fs::write(staged.join("ready"), &release.version)?;
        Ok(programs)
    })();
    if let Err(error) = std::fs::remove_dir_all(&work) {
        log::debug!("{} was not taken away: {error}", work.display());
    }
    match &result {
        Ok(programs) => log::debug!(
            "release {} is staged: {}",
            release.version,
            programs.display()
        ),
        Err(error) => log::debug!(
            "release {} was not staged{}: {error:#}",
            release.version,
            if replaced {
                " and what was staged before is gone"
            } else {
                "; what was staged before is kept"
            }
        ),
    }
    if result.is_err() && replaced {
        unstage(folder);
    }
    result
}

// ---------------------------------------------------------------------
// Putting it in place

/// Whether the gevrek in this folder was installed from the package (by
/// its `install.ps1`, which leaves `uninstall.ps1` there, or by the setup
/// program, which leaves its uninstaller): only such an install is updated.
pub fn is_packaged(install: &Path) -> bool {
    WINDOWS_PROGRAMS
        .iter()
        .all(|program| install.join(program).is_file())
        && (install.join("uninstall.ps1").is_file() || install.join("unins000.exe").is_file())
}

/// Put the programs of `package` in place of those in `install`. The ones
/// that were there are moved to `install/previous` (a program that runs
/// cannot be written over, but it can be moved aside, this one included).
/// If a step fails, what was moved is moved back and the error is returned:
/// the install is then as it was.
pub fn apply(package: &Path, install: &Path) -> Result<()> {
    for program in WINDOWS_PROGRAMS {
        if !package.join(program).is_file() {
            bail!("{} is missing from the update", program);
        }
    }
    let previous = install.join("previous");
    std::fs::create_dir_all(&previous)?;
    // The programs to put in place: all of the first kind, and of the
    // later ones those the package has.
    let programs: Vec<&str> = WINDOWS_PROGRAMS
        .iter()
        .copied()
        .chain(
            LATER_PROGRAMS
                .iter()
                .copied()
                .filter(|program| package.join(program).is_file()),
        )
        .collect();
    // Each is written beside its place first, whole, under another name.
    // Putting it in place is then two renames, done in a moment: a
    // machine that loses its power meanwhile is not left with a program
    // moved away and its new file half written.
    let beside = |program: &str| install.join(format!("{program}.new"));
    for program in &programs {
        if let Err(error) = std::fs::copy(package.join(program), beside(program)) {
            for written in &programs {
                let _ = std::fs::remove_file(beside(written));
            }
            return Err(error).with_context(|| format!("write {}", beside(program).display()));
        }
    }
    let mut moved: Vec<&str> = Vec::new();
    let mut placed: Vec<&str> = Vec::new();
    let result = (|| -> Result<()> {
        for program in programs.iter().copied() {
            let to = install.join(program);
            if to.exists() {
                let aside = previous.join(program);
                // (What is in `previous` from the update before may still
                // run, when a program was never restarted since: then it
                // cannot be replaced, and is moved out of the way itself.)
                if aside.exists() && std::fs::remove_file(&aside).is_err() {
                    log::debug!("{} still runs: it is moved out of the way", aside.display());
                    if let Err(error) =
                        std::fs::rename(&aside, previous.join(format!("{program}.old")))
                    {
                        log::debug!("{} was not moved out of the way: {error}", aside.display());
                    }
                }
                std::fs::rename(&to, &aside)
                    .with_context(|| format!("move {} aside", to.display()))?;
                moved.push(program);
            }
            std::fs::rename(beside(program), &to)
                .with_context(|| format!("put {} in place", to.display()))?;
            placed.push(program);
        }
        Ok(())
    })();
    if let Err(error) = result {
        log::debug!(
            "the update was not put in place in {}, what was moved is moved back: {error:#}",
            install.display()
        );
        for program in programs.iter() {
            let _ = std::fs::remove_file(beside(program));
            let to = install.join(program);
            if moved.contains(program) {
                let _ = std::fs::remove_file(&to);
                if let Err(back) = std::fs::rename(previous.join(program), &to) {
                    log::error!("{} could not be put back: {back}", to.display());
                }
            } else if placed.contains(program) {
                let _ = std::fs::remove_file(&to);
            }
        }
        return Err(error);
    }
    for file in BESIDE {
        if package.join(file).is_file() {
            if let Err(error) = std::fs::copy(package.join(file), install.join(file)) {
                log::debug!(
                    "{file} was not put in place in {}: {error}",
                    install.display()
                );
            }
        }
    }
    log::debug!(
        "the update was put in place in {}: {}; the programs before it are in {}",
        install.display(),
        programs.join(", "),
        previous.display()
    );
    Ok(())
}

/// Tell Windows' list of installed apps which version is installed now
/// (the entry of `install.ps1`, and the setup program's). Nothing is added
/// where there is no entry, and failing is not an error.
pub fn note_installed_version(version: &str) {
    if !cfg!(windows) {
        return;
    }
    for entry in ["gevrek", "gevrek_is1"] {
        let key = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{entry}");
        let there = quiet(Command::new(system_program("reg")).args(["query", &key]))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if there {
            let noted = quiet(Command::new(system_program("reg")).args([
                "add",
                &key,
                "/v",
                "DisplayVersion",
                "/d",
                version,
                "/f",
            ]))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
            match noted {
                Ok(status) if status.success() => {
                    log::debug!("Windows' list of apps says {version} for {entry} now")
                }
                Ok(status) => log::debug!(
                    "Windows' list of apps was not told {version} for {entry}: reg ended with {status}"
                ),
                Err(error) => {
                    log::debug!("Windows' list of apps was not told {version} for {entry}: {error}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("gevrek-update-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A folder with the four programs, each holding `words`.
    fn programs(folder: &Path, words: &str) {
        std::fs::create_dir_all(folder).unwrap();
        for program in WINDOWS_PROGRAMS {
            std::fs::write(folder.join(program), words).unwrap();
        }
    }

    #[test]
    fn a_later_version_is_newer_and_nothing_else_is() {
        assert!(is_newer("0.7.0", "0.6.0"));
        assert!(is_newer("v0.10.0", "0.9.3"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(is_newer("0.6.1", "0.6"));
        assert!(!is_newer("0.6.0", "0.6.0"));
        assert!(!is_newer("0.5.9", "0.6.0"));
        // A release is later than its pre-release, not the other way round.
        assert!(is_newer("0.7.0", "0.7.0-rc.1"));
        assert!(!is_newer("0.7.0-rc.1", "0.7.0"));
        assert!(!is_newer("0.7.0-rc.2", "0.7.0-rc.1"));
        // What is no version is never newer.
        assert!(!is_newer("nightly", "0.6.0"));
        assert!(!is_newer("", "0.6.0"));
        assert!(!is_newer("1.x", "0.6.0"));
    }

    #[test]
    fn a_check_is_due_once_a_day_and_never_when_turned_off() {
        let mut settings = Settings::default();
        assert!(
            settings.automatic,
            "gevrek looks for updates unless told not to"
        );
        assert!(settings.check_due(1_000_000));
        settings.last_check = Some(1_000_000);
        assert!(!settings.check_due(1_000_000 + 3_600));
        assert!(settings.check_due(1_000_000 + CHECK_EVERY));
        // The clock was set back.
        assert!(settings.check_due(999_000));
        settings.automatic = false;
        assert!(!settings.check_due(9_000_000));
    }

    #[test]
    fn settings_are_kept_and_a_broken_file_is_the_default() {
        let folder = folder("settings");
        assert_eq!(Settings::load(&folder), Settings::default());
        let settings = Settings {
            automatic: false,
            last_check: Some(42),
        };
        settings.save(&folder).unwrap();
        assert_eq!(Settings::load(&folder), settings);
        std::fs::write(folder.join("update.json"), "{ not json").unwrap();
        assert_eq!(Settings::load(&folder), Settings::default());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_release_is_read_and_files_from_elsewhere_are_not_taken() {
        let source = Source::github();
        let text = r#"{
            "tag_name": "v0.7.0",
            "html_url": "https://github.com/ulaas/gevrek/releases/tag/v0.7.0",
            "assets": [
                {"name": "gevrek-windows-x64.zip", "browser_download_url": "https://github.com/ulaas/gevrek/releases/download/v0.7.0/gevrek-windows-x64.zip"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://example.com/SHA256SUMS.txt"},
                {"name": "gevrek-linux-x64.rpm", "browser_download_url": "https://github.com/ulaas/gevrek/releases/download/v0.7.0/gevrek-linux-x64.rpm"}
            ]
        }"#;
        let release = parse_release(text, &source).unwrap();
        assert_eq!(release.version, "0.7.0");
        assert_eq!(
            release.page,
            "https://github.com/ulaas/gevrek/releases/tag/v0.7.0"
        );
        assert_eq!(release.package.unwrap().name, "gevrek-windows-x64.zip");
        assert_eq!(
            release.sums, None,
            "a list of checksums from another site is none"
        );

        assert!(parse_release(r#"{"tag_name": "nightly"}"#, &source).is_err());
        assert!(parse_release("busy", &source).is_err());
        let bare = parse_release(
            r#"{"tag_name": "0.8.0", "html_url": "http://elsewhere"}"#,
            &source,
        )
        .unwrap();
        assert_eq!(bare.page, RELEASES_PAGE);
        assert_eq!(bare.package, None);
    }

    #[test]
    fn the_checksum_of_a_file_is_found_in_the_list() {
        let one = "a".repeat(64);
        let other = "B".repeat(64);
        let sums =
            format!("{one}  gevrek-linux-x64.rpm\n{other} *gevrek-windows-x64.zip\nshort  x.zip\n");
        assert_eq!(sum_of(&sums, "gevrek-linux-x64.rpm"), Some(one));
        assert_eq!(
            sum_of(&sums, "gevrek-windows-x64.zip"),
            Some("b".repeat(64))
        );
        assert_eq!(sum_of(&sums, "x.zip"), None);
        assert_eq!(sum_of(&sums, "gevrek.zip"), None);
    }

    #[test]
    fn the_sha256_of_a_file_is_the_known_one() {
        let folder = folder("sha");
        let file = folder.join("abc");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(
            sha256_of(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn only_an_install_from_the_package_is_one() {
        let install = folder("packaged");
        assert!(!is_packaged(&install));
        programs(&install, "old");
        assert!(
            !is_packaged(&install),
            "a build copied there by hand is not"
        );
        std::fs::write(install.join("uninstall.ps1"), "").unwrap();
        assert!(is_packaged(&install));
        std::fs::remove_file(install.join("uninstall.ps1")).unwrap();
        std::fs::write(install.join("unins000.exe"), "").unwrap();
        assert!(is_packaged(&install));
        std::fs::remove_file(install.join("gevrek-wm.exe")).unwrap();
        assert!(!is_packaged(&install));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn applying_puts_the_new_programs_in_place_and_keeps_the_old() {
        let root = folder("apply");
        let (package, install) = (root.join("package"), root.join("install"));
        programs(&package, "new");
        std::fs::write(package.join("README.txt"), "new words").unwrap();
        programs(&install, "old");
        // From the update before.
        programs(&install.join("previous"), "older");

        apply(&package, &install).unwrap();
        for program in WINDOWS_PROGRAMS {
            assert_eq!(
                std::fs::read_to_string(install.join(program)).unwrap(),
                "new"
            );
            assert_eq!(
                std::fs::read_to_string(install.join("previous").join(program)).unwrap(),
                "old"
            );
        }
        assert_eq!(
            std::fs::read_to_string(install.join("README.txt")).unwrap(),
            "new words"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_update_that_lacks_a_program_changes_nothing() {
        let root = folder("incomplete");
        let (package, install) = (root.join("package"), root.join("install"));
        programs(&package, "new");
        std::fs::remove_file(package.join("gevrek-shell.exe")).unwrap();
        programs(&install, "old");
        assert!(apply(&package, &install).is_err());
        for program in WINDOWS_PROGRAMS {
            assert_eq!(
                std::fs::read_to_string(install.join(program)).unwrap(),
                "old"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_step_puts_everything_back() {
        let root = folder("rollback");
        let (package, install) = (root.join("package"), root.join("install"));
        programs(&package, "new");
        programs(&install, "old");
        // The third program cannot be moved aside: a folder with something
        // in it is in its place in `previous`, and another where that one
        // would be moved out of the way to.
        let blocked = install.join("previous").join(WINDOWS_PROGRAMS[2]);
        std::fs::create_dir_all(blocked.join("in the way")).unwrap();
        let also = install
            .join("previous")
            .join(format!("{}.old", WINDOWS_PROGRAMS[2]));
        std::fs::create_dir_all(also.join("in the way")).unwrap();
        assert!(apply(&package, &install).is_err());
        for program in WINDOWS_PROGRAMS {
            assert_eq!(
                std::fs::read_to_string(install.join(program)).unwrap(),
                "old",
                "{program} is as it was"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_is_staged_counts_only_when_it_is_complete() {
        let folder = folder("staged");
        assert_eq!(staged(&folder), None);
        let package = staged_folder(&folder).join(WINDOWS_PACKAGE);
        programs(&package, "new");
        assert_eq!(
            staged(&folder),
            None,
            "not without the mark that is written last"
        );
        std::fs::write(staged_folder(&folder).join("ready"), "0.7.0\n").unwrap();
        assert_eq!(
            staged(&folder),
            Some(("0.7.0".to_string(), package.clone()))
        );
        std::fs::remove_file(package.join("gevrek.exe")).unwrap();
        assert_eq!(staged(&folder), None);
        unstage(&folder);
        assert!(!staged_folder(&folder).exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The whole way with the system's own curl and tar, against a release
    /// that is a folder on this machine.
    #[cfg(windows)]
    #[test]
    fn a_release_in_a_folder_is_checked_staged_and_refused_when_it_is_another_file() {
        let root = folder("feed");
        let feed = root.join("feed");
        let built = root.join("built").join(WINDOWS_PACKAGE);
        programs(&built, "new");
        std::fs::create_dir_all(&feed).unwrap();
        let zip = feed.join(format!("{WINDOWS_PACKAGE}.zip"));
        let packed = Command::new(system_program("tar"))
            .arg("-a")
            .arg("-cf")
            .arg(&zip)
            .arg("-C")
            .arg(root.join("built"))
            .arg(WINDOWS_PACKAGE)
            .status()
            .unwrap();
        assert!(packed.success());
        let address =
            |file: &Path| format!("file:///{}", file.display().to_string().replace('\\', "/"));
        let base = format!("{}/", address(&feed));
        let source = Source {
            api: format!("{base}latest.json"),
            downloads: base.clone(),
            https_only: false,
        };
        std::fs::write(
            feed.join("latest.json"),
            serde_json::json!({
                "tag_name": "v99.0.0",
                "assets": [
                    {"name": format!("{WINDOWS_PACKAGE}.zip"), "browser_download_url": format!("{base}{WINDOWS_PACKAGE}.zip")},
                    {"name": SUMS, "browser_download_url": format!("{base}{SUMS}")},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let state = root.join("state");

        // The list names another file's sum: nothing is staged.
        std::fs::write(
            feed.join(SUMS),
            format!("{}  {WINDOWS_PACKAGE}.zip\n", "0".repeat(64)),
        )
        .unwrap();
        let release = latest(&source, &state).unwrap();
        assert_eq!(release.version, "99.0.0");
        assert!(is_newer(&release.version, current()));
        let refused = stage(&source, &release, &state).unwrap_err().to_string();
        assert!(
            refused.contains("is not the file the release names"),
            "{refused}"
        );
        assert_eq!(staged(&state), None);

        // With the right sum it is staged, and can be applied.
        std::fs::write(
            feed.join(SUMS),
            format!("{}  {WINDOWS_PACKAGE}.zip\n", sha256_of(&zip).unwrap()),
        )
        .unwrap();
        let package = stage(&source, &release, &state).unwrap();
        assert_eq!(
            staged(&state),
            Some(("99.0.0".to_string(), package.clone()))
        );
        assert!(
            !state.join("download").exists(),
            "the downloads are not kept"
        );
        let install = root.join("install");
        programs(&install, "old");
        apply(&package, &install).unwrap();
        assert_eq!(
            std::fs::read_to_string(install.join("gevrek-wm.exe")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
