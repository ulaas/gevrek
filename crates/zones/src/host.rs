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
                    // shows nothing; its zone stays.
                    if let Ok(watch) = backend.watch(&location) {
                        self.watches.insert(directory, watch);
                    }
                    let items = backend.list(&location).unwrap_or_default();
                    queue.extend(core.event(BackendEvent::Listed { location, items }, now));
                }
                BackendRequest::Unwatch { directory } => {
                    if let Some(watch) = self.watches.remove(&directory) {
                        backend.unwatch(watch);
                    }
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
        if let Some(watch) = self.desktop.take() {
            backend.unwatch(watch);
        }
        self.started = false;
    }
}
