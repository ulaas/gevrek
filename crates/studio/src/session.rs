//! The editor's view of the configuration it shares with the window manager
//! (docs/REMEDIATION_PLAN.md, Phase 3 step 1).
//!
//! The WM owns the configuration and numbers every change with a revision.
//! The editor keeps:
//!
//! - `base`: the configuration it last got from, or successfully applied to,
//!   the WM, with its revision;
//! - `draft`: the user's unsaved edits on top of `base`, if any;
//! - `remote`: a newer configuration from the WM that arrived while there
//!   was a draft, i.e. a conflict the user has to resolve.
//!
//! Applying sends the draft with `base_revision`; the WM refuses it if the
//! running revision is different, so two editors (or an editor and the CLI,
//! or a hand edit of the file) never silently overwrite each other. Change
//! events carry the revision and the id of the client that made the change,
//! so the editor ignores the echo of its own apply.
//!
//! This module holds no I/O: the host calls it with what happened and does
//! what it answers.

use gevrek_model::Config;

/// What the editor sends to apply its draft.
#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub config: Config,
    /// The revision the draft was edited from.
    pub base_revision: u64,
    pub client_id: String,
}

/// What a change event from the WM means for this editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteNotice {
    /// The echo of this editor's own apply.
    OwnEcho,
    /// A revision the editor already has.
    AlreadyKnown,
    /// Someone else changed the configuration: fetch it.
    Fetch,
}

/// What happened when a fetched configuration was taken in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// Nothing newer than what the editor has.
    UpToDate,
    /// No unsaved edits: the editor now shows the fetched configuration.
    FastForwarded,
    /// There are unsaved edits: the fetched configuration is kept aside
    /// until the user chooses (`keep_mine` or `take_theirs`).
    Conflict,
}

#[derive(Debug, Clone)]
pub struct Session {
    client_id: String,
    base: Config,
    base_revision: u64,
    draft: Option<Config>,
    remote: Option<(Config, u64)>,
    /// The revision of this editor's own last apply.
    own_revision: Option<u64>,
}

impl Session {
    /// A session starting from the configuration fetched from the WM.
    pub fn new(client_id: impl Into<String>, config: Config, revision: u64) -> Self {
        Self {
            client_id: client_id.into(),
            base: config,
            base_revision: revision,
            draft: None,
            remote: None,
            own_revision: None,
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The revision the editor's configuration is based on.
    pub fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// The configuration the editor shows: the draft if there is one.
    pub fn config(&self) -> &Config {
        self.draft.as_ref().unwrap_or(&self.base)
    }

    /// The configuration last known to run in the WM.
    pub fn base(&self) -> &Config {
        &self.base
    }

    /// There are edits the WM does not have.
    pub fn is_dirty(&self) -> bool {
        self.draft.is_some()
    }

    /// The WM has a newer configuration than the one the draft is based on.
    pub fn has_conflict(&self) -> bool {
        self.remote.is_some()
    }

    /// The newer configuration and its revision, while there is a conflict.
    pub fn remote(&self) -> Option<(&Config, u64)> {
        self.remote
            .as_ref()
            .map(|(config, revision)| (config, *revision))
    }

    /// The configuration to edit. The first edit starts a draft.
    pub fn edit(&mut self) -> &mut Config {
        self.draft.get_or_insert_with(|| self.base.clone())
    }

    /// Replace the draft with `config` (the editor builds a whole
    /// configuration from its widgets).
    pub fn set_draft(&mut self, config: Config) {
        self.draft = Some(config);
    }

    /// Drop the unsaved edits.
    pub fn discard_draft(&mut self) {
        self.draft = None;
    }

    /// The request that applies the draft. `None` when there is nothing to
    /// apply, or while a conflict is unresolved (the base is known stale).
    pub fn begin_apply(&self) -> Option<ApplyRequest> {
        if self.has_conflict() {
            return None;
        }
        Some(ApplyRequest {
            config: self.draft.clone()?,
            base_revision: self.base_revision,
            client_id: self.client_id.clone(),
        })
    }

    /// The WM applied the draft sent with `begin_apply` as `revision`.
    pub fn applied(&mut self, revision: u64) {
        if let Some(draft) = self.draft.take() {
            self.base = draft;
        }
        self.base_revision = revision;
        self.own_revision = Some(revision);
    }

    /// The WM applied this editor's canvas graph (`PushGraph`) as
    /// `revision`. Returns whether the session is in step with the WM: if
    /// the push did not directly follow the revision the editor had, some
    /// other change lies in between and the host fetches the configuration.
    pub fn graph_pushed(&mut self, graph: gevrek_model::graph::NodeGraph, revision: u64) -> bool {
        self.own_revision = Some(revision);
        if self.remote.is_some() || revision != self.base_revision + 1 {
            return false;
        }
        self.base.node_graph = graph;
        self.base_revision = revision;
        true
    }

    /// The editor reconnected and could not read the WM's configuration.
    /// The WM may have restarted, and its revisions start again, so the
    /// revision the editor had means nothing: whatever is fetched next is
    /// taken as newer.
    pub fn revision_lost(&mut self) {
        self.base_revision = 0;
        self.own_revision = None;
    }

    /// A change event arrived. Decide whether to fetch the configuration.
    pub fn on_remote_revision(&self, revision: u64, origin_client_id: &str) -> RemoteNotice {
        if origin_client_id == self.client_id && self.own_revision == Some(revision) {
            return RemoteNotice::OwnEcho;
        }
        let known = self
            .remote
            .as_ref()
            .map_or(self.base_revision, |(_, remote)| *remote);
        if revision <= known {
            RemoteNotice::AlreadyKnown
        } else {
            RemoteNotice::Fetch
        }
    }

    /// The configuration fetched from the WM, with its revision (after a
    /// `Fetch` notice, a refused apply, or a reconnect).
    pub fn remote_fetched(&mut self, config: Config, revision: u64) -> Fetched {
        if revision <= self.base_revision {
            return Fetched::UpToDate;
        }
        if self.draft.is_none() {
            self.base = config;
            self.base_revision = revision;
            self.remote = None;
            return Fetched::FastForwarded;
        }
        self.remote = Some((config, revision));
        Fetched::Conflict
    }

    /// Resolve a conflict by keeping the draft: it is now based on the WM's
    /// newer configuration, and the next apply replaces that one knowingly.
    pub fn keep_mine(&mut self) {
        if let Some((config, revision)) = self.remote.take() {
            self.base = config;
            self.base_revision = revision;
        }
    }

    /// Resolve a conflict by dropping the draft and showing the WM's newer
    /// configuration.
    pub fn take_theirs(&mut self) {
        if let Some((config, revision)) = self.remote.take() {
            self.base = config;
            self.base_revision = revision;
            self.draft = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(history_size: usize) -> Config {
        let mut config = Config::default();
        config.desktop.focus.history_size = history_size;
        config
    }

    fn history(config: &Config) -> usize {
        config.desktop.focus.history_size
    }

    #[test]
    fn an_edit_starts_a_draft_and_apply_makes_it_the_base() {
        let mut session = Session::new("me", config(1), 5);
        assert!(!session.is_dirty());
        assert!(session.begin_apply().is_none(), "nothing to apply");

        session.edit().desktop.focus.history_size = 2;
        assert!(session.is_dirty());
        assert_eq!(history(session.config()), 2);
        assert_eq!(history(session.base()), 1);

        let request = session.begin_apply().unwrap();
        assert_eq!(
            (request.base_revision, request.client_id.as_str()),
            (5, "me")
        );
        assert_eq!(history(&request.config), 2);

        session.applied(6);
        assert!(!session.is_dirty());
        assert_eq!((history(session.base()), session.base_revision()), (2, 6));
    }

    #[test]
    fn the_echo_of_an_own_apply_is_ignored() {
        let mut session = Session::new("me", config(1), 5);
        session.edit().desktop.focus.history_size = 2;
        session.applied(6);
        assert_eq!(session.on_remote_revision(6, "me"), RemoteNotice::OwnEcho);
        assert_eq!(
            session.on_remote_revision(6, "other"),
            RemoteNotice::AlreadyKnown
        );
        assert_eq!(
            session.on_remote_revision(4, ""),
            RemoteNotice::AlreadyKnown
        );
        assert_eq!(session.on_remote_revision(7, "other"), RemoteNotice::Fetch);
        // A change by this client that is not its last apply (another window
        // of the same editor, a replayed id) is still fetched.
        assert_eq!(session.on_remote_revision(7, "me"), RemoteNotice::Fetch);
    }

    #[test]
    fn a_remote_change_without_local_edits_fast_forwards() {
        let mut session = Session::new("me", config(1), 5);
        assert_eq!(session.remote_fetched(config(9), 6), Fetched::FastForwarded);
        assert_eq!((history(session.config()), session.base_revision()), (9, 6));
        assert_eq!(session.remote_fetched(config(8), 6), Fetched::UpToDate);
        assert_eq!(history(session.config()), 9);
    }

    #[test]
    fn a_remote_change_during_local_edits_is_a_conflict_until_resolved() {
        let mut session = Session::new("me", config(1), 5);
        session.edit().desktop.focus.history_size = 2;

        assert_eq!(session.remote_fetched(config(9), 6), Fetched::Conflict);
        assert!(session.has_conflict());
        assert_eq!(history(session.config()), 2, "the draft is untouched");
        assert_eq!(session.remote().map(|(c, r)| (history(c), r)), Some((9, 6)));
        assert!(
            session.begin_apply().is_none(),
            "no apply on a known-stale base"
        );
        // A further remote change replaces the pending one.
        assert_eq!(
            session.on_remote_revision(6, "other"),
            RemoteNotice::AlreadyKnown
        );
        assert_eq!(session.on_remote_revision(7, "other"), RemoteNotice::Fetch);
        assert_eq!(session.remote_fetched(config(10), 7), Fetched::Conflict);

        // Keep mine: the draft now sits on the newest revision.
        let mut mine = session.clone();
        mine.keep_mine();
        assert!(!mine.has_conflict() && mine.is_dirty());
        let request = mine.begin_apply().unwrap();
        assert_eq!((history(&request.config), request.base_revision), (2, 7));

        // Take theirs: the draft is gone.
        session.take_theirs();
        assert!(!session.has_conflict() && !session.is_dirty());
        assert_eq!(
            (history(session.config()), session.base_revision()),
            (10, 7)
        );
    }

    #[test]
    fn a_refused_apply_leads_to_a_conflict_after_fetching() {
        // The WM refused the apply (someone was faster). The host fetches
        // the running configuration and hands it in.
        let mut session = Session::new("me", config(1), 5);
        session.set_draft(config(2));
        let request = session.begin_apply().unwrap();
        assert_eq!(request.base_revision, 5);
        assert_eq!(session.remote_fetched(config(9), 6), Fetched::Conflict);
        session.keep_mine();
        assert_eq!(session.begin_apply().unwrap().base_revision, 6);
    }

    #[test]
    fn an_own_graph_push_moves_the_base_only_when_nothing_lies_in_between() {
        let mut graph = gevrek_model::graph::NodeGraph::new();
        graph.add_node(gevrek_model::graph::NodeType::OnStartup, [0.0, 0.0]);

        let mut session = Session::new("me", config(1), 5);
        assert!(session.graph_pushed(graph.clone(), 6));
        assert_eq!(session.base_revision(), 6);
        assert_eq!(session.base().node_graph.nodes.len(), 1);
        assert_eq!(session.on_remote_revision(6, "me"), RemoteNotice::OwnEcho);

        // Someone else's revision 6 came first: the push is revision 7 and
        // the session must fetch instead of assuming it knows revision 7.
        let mut session = Session::new("me", config(1), 5);
        assert!(!session.graph_pushed(graph, 7));
        assert_eq!(session.base_revision(), 5);
        assert_eq!(session.on_remote_revision(6, "other"), RemoteNotice::Fetch);
    }

    #[test]
    fn after_a_lost_revision_whatever_the_wm_runs_is_taken_as_newer() {
        // The WM restarted: its revisions start again below the one the
        // editor had.
        let mut session = Session::new("me", config(1), 50);
        assert_eq!(session.remote_fetched(config(9), 2), Fetched::UpToDate);
        session.revision_lost();
        assert_eq!(session.on_remote_revision(2, ""), RemoteNotice::Fetch);
        assert_eq!(session.remote_fetched(config(9), 2), Fetched::FastForwarded);
        assert_eq!((history(session.config()), session.base_revision()), (9, 2));
    }

    #[test]
    fn discarding_the_draft_returns_to_the_base() {
        let mut session = Session::new("me", config(1), 5);
        session.edit().desktop.focus.history_size = 2;
        session.discard_draft();
        assert!(!session.is_dirty());
        assert_eq!(history(session.config()), 1);
    }
}
