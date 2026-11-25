//! Polling configuration file watcher.
//!
//! A deliberate no-dependency design: the runtime loop already ticks at
//! ~60 Hz, so once a second we stat the config file's mtime and compare it
//! with the last acknowledged value. A modification must survive two
//! consecutive polls before it is reported, which debounces non-atomic
//! editor saves (truncate-then-write, temp-file renames) without any
//! platform-specific watching code.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How often the file is statted.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub struct ConfigFileWatcher {
    path: PathBuf,
    enabled: bool,
    /// Last acknowledged modification time (`None` = file missing).
    acknowledged: Option<SystemTime>,
    /// Modification state seen at the previous poll but not yet confirmed.
    /// The outer `Option` distinguishes "no candidate" from "candidate is a
    /// deletion" (inner `None`).
    pending: Option<Option<SystemTime>>,
    last_poll: Instant,
}

impl ConfigFileWatcher {
    pub fn new(path: PathBuf, enabled: bool) -> Self {
        let acknowledged = modification_time(&path);
        log::debug!(
            "{} is {}watched for changes{}",
            path.display(),
            if enabled { "" } else { "not " },
            if acknowledged.is_none() { " (it is not there yet)" } else { "" }
        );
        Self {
            path,
            enabled,
            acknowledged,
            pending: None,
            // Allow the first poll immediately so a file changed between WM
            // start and watcher creation is noticed within one interval.
            last_poll: Instant::now().checked_sub(POLL_INTERVAL).unwrap_or_else(Instant::now),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable watching. Enabling re-acknowledges the current
    /// mtime so a stale diff accumulated while disabled does not trigger an
    /// immediate reload.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.acknowledged = modification_time(&self.path);
            self.pending = None;
        }
        if enabled != self.enabled {
            log::debug!(
                "{} is {}watched for changes from now on",
                self.path.display(),
                if enabled { "" } else { "not " }
            );
        }
        self.enabled = enabled;
    }

    /// Adopt the file's current mtime without reporting a change. Called
    /// when the configuration was (re)applied through the push channel —
    /// that flow includes the WM's own file writes, which must not feed
    /// back into the watcher as an external change.
    pub fn acknowledge(&mut self) {
        self.acknowledged = modification_time(&self.path);
        self.pending = None;
    }

    /// Poll the file. Returns `true` at most once per actual modification:
    /// a change candidate must be stable across two consecutive polls before
    /// it is reported and acknowledged.
    pub fn poll(&mut self) -> bool {
        if !self.enabled || self.last_poll.elapsed() < POLL_INTERVAL {
            return false;
        }
        self.last_poll = Instant::now();

        let current = modification_time(&self.path);
        if current == self.acknowledged {
            self.pending = None;
            return false;
        }
        if self.pending.as_ref() == Some(&current) {
            // Stable across two polls: a real change (or a confirmed
            // deletion). Acknowledge so we do not report it again.
            self.acknowledged = current;
            self.pending = None;
            log::debug!(
                "{} was {} outside gevrek",
                self.path.display(),
                if current.is_some() { "changed" } else { "removed" }
            );
            return true;
        }
        self.pending = Some(current);
        false
    }
}

fn modification_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "gevrek-config-watch-{name}-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, "{}").unwrap();
        path
    }

    fn poll_now(watcher: &mut ConfigFileWatcher) -> bool {
        watcher.last_poll = Instant::now().checked_sub(POLL_INTERVAL).unwrap_or_else(Instant::now);
        watcher.poll()
    }

    #[test]
    fn unmodified_file_never_reports_a_change() {
        let path = temp_file("stable");
        let mut watcher = ConfigFileWatcher::new(path.clone(), true);
        assert!(!poll_now(&mut watcher));
        assert!(!poll_now(&mut watcher));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn modification_is_reported_once_after_debounce() {
        let path = temp_file("modified");
        let mut watcher = ConfigFileWatcher::new(path.clone(), true);
        // Rewrite with different content so the mtime moves forward.
        std::thread::sleep(Duration::from_millis(20));
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(b"{ \"changed\": true }").unwrap();
        }
        // First poll sees the candidate; the debounce holds it back.
        assert!(!poll_now(&mut watcher));
        // Second poll confirms the stable change.
        assert!(poll_now(&mut watcher));
        // Already acknowledged: no repeat report.
        assert!(!poll_now(&mut watcher));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn deletion_is_reported_as_a_change() {
        let path = temp_file("deleted");
        let mut watcher = ConfigFileWatcher::new(path.clone(), true);
        std::fs::remove_file(&path).unwrap();
        assert!(!poll_now(&mut watcher));
        assert!(poll_now(&mut watcher));
        assert!(!poll_now(&mut watcher));
    }

    #[test]
    fn acknowledge_suppresses_self_writes() {
        let path = temp_file("self-write");
        let mut watcher = ConfigFileWatcher::new(path.clone(), true);
        // Simulate the WM persisting the config itself (UpdateConfiguration),
        // then the channel-driven apply acknowledging the write.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, "{ \"self\": true }").unwrap();
        watcher.acknowledge();
        assert!(!poll_now(&mut watcher));
        assert!(!poll_now(&mut watcher));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn disabled_watcher_stays_quiet_and_reenable_acknowledges() {
        let path = temp_file("disabled");
        let mut watcher = ConfigFileWatcher::new(path.clone(), false);
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, "{ \"changed\": true }").unwrap();
        assert!(!watcher.poll());
        // Enabling adopts the current mtime: the change made while disabled
        // is not reported.
        watcher.set_enabled(true);
        assert!(!poll_now(&mut watcher));
        assert!(!poll_now(&mut watcher));
        // But a change made after enabling is.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, "{ \"changed\": \"again\" }").unwrap();
        assert!(!poll_now(&mut watcher));
        assert!(poll_now(&mut watcher));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_at_start_watches_for_creation() {
        let path = std::env::temp_dir().join(format!(
            "gevrek-config-watch-created-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        let mut watcher = ConfigFileWatcher::new(path.clone(), true);
        assert!(!poll_now(&mut watcher));
        std::fs::write(&path, "{}").unwrap();
        assert!(!poll_now(&mut watcher));
        assert!(poll_now(&mut watcher));
        let _ = std::fs::remove_file(path);
    }
}
