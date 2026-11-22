//! The configuration: what the server is asked to run, what is written in
//! the file, and what runs, published.
//!
//! A request carries the revision it was made against; one made against a
//! configuration that no longer runs is refused, so that two editors do not
//! undo each other's changes without knowing. A change made here (a toggle
//! from a key) is published at the end of the pass, or before a request is
//! judged, so that it has a revision of its own.

use super::reload::Reload;
use super::published::Published;
use super::WmHost;
use gevrek_model::config::Config;

/// The host's handling of the configuration, for every backend.
pub trait Configuration: WmHost {
    /// Take a configuration request from the server, or an edit of the
    /// file, and apply it. Runs while the WM is paused too: a change made
    /// in the editor does not wait for the resume.
    fn apply_configuration_changes(&mut self) {
        let mut request = {
            let host = self.parts().host;
            host.config_receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok())
        };
        if let Some(base) = request.as_ref().and_then(|request| request.base_revision()) {
            // A change made here since the last publish gets its revision
            // first; the request is judged against what really runs.
            self.publish_config_if_changed();
            let parts = self.parts();
            let running = parts
                .host
                .config_publisher
                .as_ref()
                .map(|publisher| publisher.revision());
            if let Some(running) = running.filter(|running| *running != base) {
                log::info!(
                    "Refusing a configuration based on revision {base}: revision {running} is running"
                );
                // The server already saved the refused configuration; the
                // running one is put back so the watcher does not load it.
                if let Some(watcher) = &mut parts.host.watcher {
                    if let Err(error) = parts.config.save(watcher.path()) {
                        log::warn!("Could not restore the configuration file: {error}");
                    }
                    watcher.acknowledge();
                }
                if let Some(publisher) = &parts.host.config_publisher {
                    publisher.refuse_request();
                }
                request = None;
            }
        }
        let from_request = request.is_some();
        let new_config = {
            let parts = self.parts();
            match request {
                Some(request) => {
                    // The server wrote the file before asking: not an edit
                    // from outside.
                    if let Some(watcher) = &mut parts.host.watcher {
                        watcher.acknowledge();
                    }
                    Some(request.resolve(parts.config))
                }
                None => parts.host.watcher.as_mut().and_then(|watcher| {
                    if !watcher.poll() {
                        return None;
                    }
                    match Config::load(watcher.path()) {
                        Ok(config) => {
                            log::info!(
                                "Configuration file changed on disk; reloading {}",
                                watcher.path().display()
                            );
                            Some(config)
                        }
                        Err(error) => {
                            log::warn!(
                                "Configuration file changed but could not be loaded: {error}"
                            );
                            None
                        }
                    }
                }),
            }
        };
        let Some(new_config) = new_config else {
            return;
        };
        self.noted_work();
        self.reload_configuration(new_config);
        // The state as it is now, before anybody is told that the
        // configuration is: who made the change asks for the state as soon
        // as it hears, and is to get what the change made of it (a
        // workspace's new title, on its tab).
        self.publish_state(Some("configuration".to_string()));
        if from_request {
            // Its own publish, right away: the server waits for this
            // request, and its revision is not to be shared with a toggle
            // made later in the same pass.
            let parts = self.parts();
            parts.host.config_dirty = false;
            if let Some(publisher) = &parts.host.config_publisher {
                let revision = publisher.publish_request(parts.config);
                log::debug!("Published requested configuration as revision {revision}");
            }
        }
    }

    /// Publish the configuration if it changed here since it was last.
    fn publish_config_if_changed(&mut self) {
        let parts = self.parts();
        if !std::mem::take(&mut parts.host.config_dirty) {
            return;
        }
        if let Some(publisher) = &parts.host.config_publisher {
            let revision = publisher.publish(parts.config);
            log::debug!("Published running configuration revision {revision}");
        }
    }

    /// Write the running configuration to the file it was read from (when
    /// there is one), as the file's own content, not as an edit from
    /// outside.
    fn save_running_configuration(&mut self) {
        let parts = self.parts();
        if let Some(watcher) = &mut parts.host.watcher {
            if let Err(error) = parts.config.save(watcher.path()) {
                log::warn!("Could not write the configuration file: {error}");
            }
            watcher.acknowledge();
        }
    }
}

impl<T: WmHost + ?Sized> Configuration for T {}
