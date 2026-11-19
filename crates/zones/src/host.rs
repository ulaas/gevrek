//! Between the core and a backend: what the backend tells is given to the
//! core, and what the core asks is asked of the backend. This is the one
//! place a backend is called from, so whoever runs the zones (the shell)
//! and a test do the same thing.
//!
//! What comes back from every call are the effects that are left for the
//! caller: what to tell a renderer and listeners, and that the
//! configuration or the state should be written.

use crate::backend::{
    BackendEvent, BackendRequest, IconHandle, IconRequest, Location, WatchId, ZonesBackend,
};
use crate::config::SmartSource;
use crate::core::{Command, Effect, ZonesCore};
use std::collections::{BTreeMap, VecDeque};

/// How often what the backend tells is passed on in one go, at most:
/// something that answers every event with another must not hold the
/// caller for ever.
const ROUNDS: usize = 64;

/// What is kept between the calls: the watches that were started.
#[derive(Debug, Default)]
pub struct Host {
    watches: BTreeMap<String, WatchId>,
    sources: BTreeMap<SmartSource, WatchId>,
    desktop: Option<WatchId>,
    /// The backend was asked what there is, and watches.
    started: bool,
    /// The pictures that came since they were last taken.
    icons: Vec<(IconRequest, Option<IconHandle>)>,
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tell the core of the displays and of what is on the desktop, start
    /// watching the desktop, and do what the core asks for at the start.
    /// With the zones switched off nothing is asked of the backend at all;
    /// it is done when they are switched on (`set_config`).
    pub fn start<B: ZonesBackend + ?Sized>(&mut self, core: &mut ZonesCore, backend: &mut B, now: u64) -> Vec<Effect> {
        if !core.enabled() || self.started {
            return Vec::new();
        }
        self.started = true;
        let mut left = Vec::new();
        let displays = BackendEvent::Displays {
            displays: backend.displays(),
            reserved: backend.reserved_areas(),
        };
        let effects = core.event(displays, now);
        left.extend(self.carry_out(core, backend, effects, now));
        // Watching begins before the listing, so that nothing that comes
        // between the two is missed.
        self.desktop = backend.watch(&Location::Desktop).ok();
        if let Ok(items) = backend.list(&Location::Desktop) {
            let effects = core.event(BackendEvent::Listed { location: Location::Desktop, items }, now);
            left.extend(self.carry_out(core, backend, effects, now));
        }
        let effects = core.start();
        left.extend(self.carry_out(core, backend, effects, now));
        // Where the system can say whether the screen is shown to others,
        // it is asked once; from then on it tells by itself.
        if let Some(shared) = backend.screen_shared() {
            let effects = core.event(BackendEvent::ScreenShared { shared }, now);
            left.extend(self.carry_out(core, backend, effects, now));
        }
        left.extend(self.pump(core, backend, now));
        left
    }

    /// Another configuration is in force. Zones that are switched on by
    /// it start as at the start; zones that are switched off let go of
    /// everything they watched.
    pub fn set_config<B: ZonesBackend + ?Sized>(
        &mut self,
        core: &mut ZonesCore,
        backend: &mut B,
        config: crate::config::ZonesConfig,
        now: u64,
    ) -> Vec<Effect> {
        let effects = core.set_config(config);
        let mut left = self.carry_out(core, backend, effects, now);
        if core.enabled() {
            left.extend(self.start(core, backend, now));
        } else {
            self.stop(backend);
        }
        left
    }

    /// Carry out a command, with everything it asks of the backend.
    pub fn command<B: ZonesBackend + ?Sized>(
        &mut self,
        core: &mut ZonesCore,
        backend: &mut B,
        command: Command,
        now: u64,
    ) -> Vec<Effect> {
        let effects = core.command(command, now);
        let mut left = self.carry_out(core, backend, effects, now);
        left.extend(self.pump(core, backend, now));
        left
    }

    /// Pass on what the backend has to tell. Called when the backend
    /// wakes its host.
    pub fn pump<B: ZonesBackend + ?Sized>(&mut self, core: &mut ZonesCore, backend: &mut B, now: u64) -> Vec<Effect> {
        let mut left = Vec::new();
        if !self.started {
            return left;
        }
        for _ in 0..ROUNDS {
            let events = backend.drain_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                // (For whoever draws: the core has no use for pictures.)
                if let BackendEvent::IconReady { request, icon } = &event {
                    self.icons.push((*request, *icon));
                }
                let effects = core.event(event, now);
                left.extend(self.carry_out(core, backend, effects, now));
            }
        }
        left
    }

    /// Ask the backend what the effects ask of it, tell the core the
    /// answers, and so on until nothing is asked. Returns the effects
    /// that are not for the backend.
    fn carry_out<B: ZonesBackend + ?Sized>(
        &mut self,
        core: &mut ZonesCore,
        backend: &mut B,
        effects: Vec<Effect>,
        now: u64,
    ) -> Vec<Effect> {
        let mut left = Vec::new();
        let mut queue: VecDeque<Effect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            let request = match effect {
                Effect::Ask(request) => request,
                other => {
                    left.push(other);
                    continue;
                }
            };
            match request {
                BackendRequest::Watch { directory } => {
                    let location = Location::Directory { path: directory.clone() };
                    // A folder that is not there (a drive not plugged in)
                    // cannot be watched; its zone stays, and says so.
                    if let Ok(watch) = backend.watch(&location) {
                        self.watches.insert(directory, watch);
                    }
                    queue.extend(Self::listing(core, backend, location, now));
                }
                BackendRequest::Relist { directory } => {
                    let location = Location::Directory { path: directory.clone() };
                    // It may be there now, where it was not before.
                    if let std::collections::btree_map::Entry::Vacant(unwatched) = self.watches.entry(directory) {
                        if let Ok(watch) = backend.watch(&location) {
                            unwatched.insert(watch);
                        }
                    }
                    queue.extend(Self::listing(core, backend, location, now));
                }
                BackendRequest::WatchSource { source } => {
                    if let Ok(watch) = backend.watch_source(&source) {
                        self.sources.insert(source.clone(), watch);
                    }
                    queue.extend(Self::source(core, backend, source, now));
                }
                BackendRequest::ListSource { source } => {
                    queue.extend(Self::source(core, backend, source, now));
                }
                BackendRequest::UnwatchSource { source } => {
                    if let Some(watch) = self.sources.remove(&source) {
                        backend.unwatch(watch);
                    }
                }
                BackendRequest::Unwatch { directory } => {
                    if let Some(watch) = self.watches.remove(&directory) {
                        backend.unwatch(watch);
                    }
                }
                BackendRequest::Cancel { request } => backend.cancel_operation(request),
                // An operation the backend carries out by itself ends
                // later, by an event; the core is told that it has begun.
                BackendRequest::Perform { request, operation } if backend.begin_operation(request, &operation) => {
                    queue.extend(core.event(BackendEvent::OperationBegun { request }, now));
                }
                BackendRequest::Perform { request, operation } => {
                    let done = match backend.perform(&operation) {
                        Ok(undo) => BackendEvent::OperationDone { request, undo: Some(undo), error: None },
                        Err(error) => BackendEvent::OperationDone { request, undo: None, error: Some(error) },
                    };
                    queue.extend(core.event(done, now));
                }
            }
        }
        left
    }

    /// List a location for the core: at once where the backend lists so,
    /// or begun now and told later (`drain_events`) where it may be slow.
    /// What cannot be listed is said as that, not as an empty folder.
    fn listing<B: ZonesBackend + ?Sized>(
        core: &mut ZonesCore,
        backend: &mut B,
        location: Location,
        now: u64,
    ) -> Vec<Effect> {
        if backend.begin_listing(&location) {
            return Vec::new();
        }
        let event = match backend.list(&location) {
            Ok(items) => BackendEvent::Listed { location, items },
            Err(error) => BackendEvent::ListingFailed { location, error },
        };
        core.event(event, now)
    }

    /// Ask the backend what a source has, for the core.
    fn source<B: ZonesBackend + ?Sized>(
        core: &mut ZonesCore,
        backend: &mut B,
        source: SmartSource,
        now: u64,
    ) -> Vec<Effect> {
        let event = match backend.list_source(&source) {
            Ok(items) => BackendEvent::SourceListed { source, items },
            Err(error) => BackendEvent::SourceFailed { source, error },
        };
        core.event(event, now)
    }

    /// The pictures the backend said are ready since this was last
    /// asked, each with the request it answers; without a handle where
    /// there is none. For the renderer, which asked for them.
    pub fn take_icons(&mut self) -> Vec<(IconRequest, Option<IconHandle>)> {
        std::mem::take(&mut self.icons)
    }

    /// Stop watching everything (the zones are switched off, the program
    /// ends).
    pub fn stop<B: ZonesBackend + ?Sized>(&mut self, backend: &mut B) {
        for (_, watch) in std::mem::take(&mut self.watches) {
            backend.unwatch(watch);
        }
        for (_, watch) in std::mem::take(&mut self.sources) {
            backend.unwatch(watch);
        }
        if let Some(watch) = self.desktop.take() {
            backend.unwatch(watch);
        }
        self.started = false;
    }
}
