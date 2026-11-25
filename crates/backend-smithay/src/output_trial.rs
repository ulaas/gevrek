//! New display settings on trial.
//!
//! A mode the display cannot show, or the only display switched off, would
//! leave a person in front of a dark screen with no way to put it right.
//! So a change of `desktop.outputs` that can do that (`needs_trial`: a
//! mode, a display on or off, a turned picture) is applied and then waits:
//! the command `output_trial` with `keep` (`gevrek output-trial keep`)
//! makes it stay; with `revert`, or after fifteen seconds without an
//! answer, the settings from before are put back, in the running
//! configuration and in the file.
//!
//! Only where the backend applies such changes while it runs
//! (`State::live_outputs`). Subscribers are told (`output.trial`), so a
//! settings window can ask.

use crate::state::State;
use gevrek_engine::executor::RuntimeEventKind;
use gevrek_host::wm::configuration::Configuration as _;
use gevrek_host::wm::reload::Reload as _;
use gevrek_model::command::TrialAnswer;
use gevrek_model::outputs::OutputProfile;
use std::time::{Duration, Instant};

/// How long new settings wait to be kept.
const TRIAL: Duration = Duration::from_secs(15);

pub struct OutputTrial {
    /// `desktop.outputs` as it was before the first change on trial.
    previous: Vec<OutputProfile>,
    until: Instant,
    /// The settings are being put back: that change is not a trial.
    answering: bool,
}

/// The length of a trial; `GEVREK_OUTPUT_TRIAL_SECONDS` sets another (the
/// checks do not wait fifteen seconds).
fn length() -> Duration {
    std::env::var("GEVREK_OUTPUT_TRIAL_SECONDS")
        .ok()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map_or(TRIAL, Duration::from_secs_f64)
}

impl State {
    /// `desktop.outputs` changed from `old` to what the configuration has
    /// now: start a trial when the change calls for one.
    pub(crate) fn outputs_changed(&mut self, old: &[OutputProfile]) {
        self.output_rescan = true;
        if !self.live_outputs
            || self
                .output_trial
                .as_ref()
                .is_some_and(|trial| trial.answering)
        {
            return;
        }
        let new = &self.config.desktop.outputs;
        // A change on top of one that is on trial: what counts is still
        // what was there before the first.
        let before = self
            .output_trial
            .take()
            .map_or_else(|| old.to_vec(), |trial| trial.previous);
        if !gevrek_model::outputs::needs_trial(&before, new) {
            return;
        }
        let length = length();
        log::info!(
            "New display settings are on trial for {:.0} seconds: `gevrek output-trial keep` keeps them",
            length.as_secs_f64()
        );
        self.output_trial = Some(OutputTrial {
            previous: before,
            until: Instant::now() + length,
            answering: false,
        });
        self.emit(
            RuntimeEventKind::OutputTrial,
            None,
            vec![
                ("state".to_string(), "started".to_string()),
                (
                    "seconds".to_string(),
                    format!("{:.0}", length.as_secs_f64()),
                ),
            ],
        );
    }

    /// A trial nobody answered in time ends by itself.
    pub(crate) fn tick_output_trial(&mut self) {
        let over = self
            .output_trial
            .as_ref()
            .is_some_and(|trial| !trial.answering && Instant::now() >= trial.until);
        if over {
            log::warn!("The new display settings were not kept: going back to the ones before");
            self.answer_output_trial(TrialAnswer::Revert);
        }
    }

    /// Keep the settings on trial, or put the ones from before back.
    /// `false` when nothing is on trial.
    pub fn answer_output_trial(&mut self, answer: TrialAnswer) -> bool {
        let Some(mut trial) = self.output_trial.take() else {
            return false;
        };
        let state = match answer {
            TrialAnswer::Keep => {
                log::info!("The new display settings are kept");
                "kept"
            }
            TrialAnswer::Revert => {
                let mut config = self.config.clone();
                config.desktop.outputs = std::mem::take(&mut trial.previous);
                trial.answering = true;
                self.output_trial = Some(trial);
                self.reload_configuration(config);
                self.output_trial = None;
                // The file has the settings that were on trial.
                self.save_running_configuration();
                "reverted"
            }
        };
        self.emit(
            RuntimeEventKind::OutputTrial,
            None,
            vec![("state".to_string(), state.to_string())],
        );
        true
    }
}
