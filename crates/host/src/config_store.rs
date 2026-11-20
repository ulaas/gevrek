//! The runtime is the single owner of the live configuration.
//!
//! It publishes every configuration it runs (startup, hot reloads from the
//! IPC channel or the file watcher, runtime toggles such as
//! `ToggleStackbar`) as a numbered snapshot. Readers, most importantly the
//! gRPC server, see exactly what is running. Before this, the server kept its
//! own copy: runtime toggles and watcher reloads never reached
//! `GetConfiguration`, and the server's next graph push silently reverted
//! them.

use gevrek_model::graph::NodeGraph;
use gevrek_model::Config;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// One published configuration. `revision` increases with every publish.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub config: Arc<Config>,
    pub revision: u64,
    /// How many `ConfigRequest`s the runtime has applied so far. The server
    /// numbers the requests it sends the same way, so it can wait for its
    /// own request rather than for any publish (a runtime toggle or a file
    /// reload also publishes).
    pub requests_applied: u64,
    /// The revision published for the request applied last.
    pub last_request_revision: u64,
}

/// A configuration change the server asks the runtime (the owner) to make.
#[derive(Debug, Clone)]
pub enum ConfigRequest {
    /// Run this complete configuration (`UpdateConfiguration`).
    Replace(Box<Config>),
    /// Run this complete configuration, but only if the runtime still runs
    /// `base_revision`. The server checks the revision too, but only the
    /// runtime sees a change it made itself (a hotkey toggle) after that
    /// check and before it applies the request.
    ReplaceFrom {
        config: Box<Config>,
        base_revision: u64,
    },
    /// Replace only the automation graph (`PushGraph`).
    ReplaceGraph(NodeGraph),
}

impl ConfigRequest {
    /// The revision the request must apply onto, if it names one.
    pub fn base_revision(&self) -> Option<u64> {
        match self {
            Self::ReplaceFrom { base_revision, .. } => Some(*base_revision),
            Self::Replace(_) | Self::ReplaceGraph(_) => None,
        }
    }

    /// The configuration to run, resolved against the one running at the
    /// moment the runtime applies the request. A graph-only request
    /// therefore keeps a runtime toggle that landed after the server read
    /// its snapshot; sending a full config built from that snapshot would
    /// have reverted it.
    pub fn resolve(self, running: &Config) -> Config {
        match self {
            Self::Replace(config) | Self::ReplaceFrom { config, .. } => *config,
            Self::ReplaceGraph(graph) => {
                let mut config = running.clone();
                config.node_graph = graph;
                config
            }
        }
    }
}

/// Read side, held by the gRPC server (cheap to clone).
pub type ConfigWatch = watch::Receiver<ConfigSnapshot>;

/// Write side, held by the runtime.
#[derive(Debug)]
pub struct ConfigPublisher {
    sender: watch::Sender<ConfigSnapshot>,
}

/// The revision of the first published configuration. Revisions start at 1
/// so that 0 can mean "no revision" on the wire (an unconditional update).
pub const FIRST_REVISION: u64 = 1;

/// What `wait_for_request` returns for a request the runtime refused
/// (never a real revision: those start at `FIRST_REVISION`).
pub const REFUSED: u64 = 0;

/// A publisher/reader pair starting at `FIRST_REVISION` with `initial`.
pub fn config_channel(initial: Config) -> (ConfigPublisher, ConfigWatch) {
    let (sender, receiver) = watch::channel(ConfigSnapshot {
        config: Arc::new(initial),
        revision: FIRST_REVISION,
        requests_applied: 0,
        last_request_revision: 0,
    });
    (ConfigPublisher { sender }, receiver)
}

impl ConfigPublisher {
    /// Publish `config` as the running configuration; returns its revision.
    pub fn publish(&self, config: &Config) -> u64 {
        let mut revision = 0;
        self.sender.send_modify(|snapshot| {
            snapshot.revision += 1;
            snapshot.config = Arc::new(config.clone());
            revision = snapshot.revision;
        });
        log::debug!("the running configuration was published as revision {revision}");
        revision
    }

    /// Answer one `ConfigRequest` with "not applied": the request counts as
    /// handled, the revision does not change, and the server's wait returns
    /// `REFUSED`.
    pub fn refuse_request(&self) {
        self.sender.send_modify(|snapshot| {
            snapshot.requests_applied += 1;
            snapshot.last_request_revision = REFUSED;
        });
        log::debug!("a configuration request was refused: it was made from an older revision");
    }

    /// Publish the configuration that results from applying one
    /// `ConfigRequest`; returns its revision.
    pub fn publish_request(&self, config: &Config) -> u64 {
        let mut revision = 0;
        self.sender.send_modify(|snapshot| {
            snapshot.revision += 1;
            snapshot.config = Arc::new(config.clone());
            snapshot.requests_applied += 1;
            snapshot.last_request_revision = snapshot.revision;
            revision = snapshot.revision;
        });
        log::debug!("a configuration request was applied and published as revision {revision}");
        revision
    }

    pub fn revision(&self) -> u64 {
        self.sender.borrow().revision
    }

    /// The configuration published last.
    pub fn current(&self) -> Arc<Config> {
        Arc::clone(&self.sender.borrow().config)
    }
}

/// Wait until the runtime has applied the request numbered `request` (the
/// first one is 1). Returns the revision published for it, or `None` on
/// timeout or if the runtime is gone. The caller must be the only sender of
/// requests while it waits (the server's write lock).
pub async fn wait_for_request(watch: &ConfigWatch, request: u64, timeout: Duration) -> Option<u64> {
    let mut watch = watch.clone();
    let wait = async move {
        watch
            .wait_for(|snapshot| snapshot.requests_applied >= request)
            .await
            .map(|snapshot| snapshot.last_request_revision)
    };
    tokio::time::timeout(timeout, wait).await.ok()?.ok()
}

/// Wait until a revision newer than `after` is published, i.e. until the
/// runtime has applied a replacement that was sent after reading `after`.
/// Returns the new snapshot, or `None` on timeout or if the runtime is gone.
pub async fn wait_for_newer(
    watch: &ConfigWatch,
    after: u64,
    timeout: Duration,
) -> Option<ConfigSnapshot> {
    let mut watch = watch.clone();
    let wait = async move {
        watch
            .wait_for(|snapshot| snapshot.revision > after)
            .await
            .map(|snapshot| snapshot.clone())
    };
    tokio::time::timeout(timeout, wait).await.ok()?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_publish_is_a_new_revision_visible_to_readers() {
        let (publisher, watch) = config_channel(Config::new());
        assert_eq!(watch.borrow().revision, FIRST_REVISION);

        let mut changed = Config::new();
        changed.desktop.appearance.border_width += 5;
        assert_eq!(publisher.publish(&changed), FIRST_REVISION + 1);
        assert_eq!(publisher.publish(&changed), FIRST_REVISION + 2);

        let snapshot = watch.borrow().clone();
        assert_eq!(snapshot.revision, FIRST_REVISION + 2);
        assert_eq!(
            snapshot.config.desktop.appearance.border_width,
            changed.desktop.appearance.border_width
        );
    }

    #[tokio::test]
    async fn a_writer_waits_for_the_runtime_to_apply_its_replacement() {
        let (publisher, watch) = config_channel(Config::new());
        let before = watch.borrow().revision;
        let runtime = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            publisher.publish(&Config::new());
            publisher
        });
        let applied = wait_for_newer(&watch, before, Duration::from_secs(5))
            .await
            .expect("runtime published");
        assert_eq!(applied.revision, before + 1);
        let _publisher = runtime.join().unwrap();
    }

    #[tokio::test]
    async fn a_refused_request_ends_the_wait_without_a_new_revision() {
        let (publisher, watch) = config_channel(Config::new());
        let before = publisher.revision();
        publisher.refuse_request();
        assert_eq!(
            wait_for_request(&watch, 1, Duration::from_millis(20)).await,
            Some(REFUSED)
        );
        assert_eq!(publisher.revision(), before);
        // The next request is numbered after the refused one.
        let applied = publisher.publish_request(&Config::new());
        assert_eq!(
            wait_for_request(&watch, 2, Duration::from_millis(20)).await,
            Some(applied)
        );
    }

    #[tokio::test]
    async fn a_request_is_not_confused_with_another_publish() {
        let (publisher, watch) = config_channel(Config::new());
        // A runtime toggle publishes: that is not the server's request.
        publisher.publish(&Config::new());
        assert!(
            wait_for_request(&watch, 1, Duration::from_millis(20))
                .await
                .is_none(),
            "a publish that is not a request must not complete the wait"
        );
        // The request is applied, and a toggle publishes right after it.
        let requested = publisher.publish_request(&Config::new());
        let toggled = publisher.publish(&Config::new());
        assert_eq!(toggled, requested + 1);
        assert_eq!(
            wait_for_request(&watch, 1, Duration::from_millis(20)).await,
            Some(requested),
            "the request's own revision, not the latest one"
        );
        assert!(wait_for_request(&watch, 2, Duration::from_millis(20))
            .await
            .is_none());
    }

    #[test]
    fn a_graph_push_keeps_a_toggle_made_after_the_server_read_its_snapshot() {
        let snapshot_read_by_server = Config::new();
        let mut graph = NodeGraph::new();
        graph.add_node(gevrek_model::graph::NodeType::OnStartup, [0.0, 0.0]);
        let request = ConfigRequest::ReplaceGraph(graph);

        // A hotkey toggles the stackbar before the runtime applies the push.
        let mut running = snapshot_read_by_server.clone();
        running.desktop.appearance.stackbar.enabled = !running.desktop.appearance.stackbar.enabled;

        let applied = request.resolve(&running);
        assert_eq!(applied.node_graph.nodes.len(), 1);
        assert_eq!(
            applied.desktop.appearance.stackbar.enabled,
            running.desktop.appearance.stackbar.enabled
        );
    }

    #[tokio::test]
    async fn waiting_gives_up_when_nothing_is_published() {
        let (_publisher, watch) = config_channel(Config::new());
        assert!(
            wait_for_newer(&watch, FIRST_REVISION, Duration::from_millis(20))
                .await
                .is_none()
        );
    }
}
