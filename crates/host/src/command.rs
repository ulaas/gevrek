//! What the IPC server asks the WM loop to do, and what the loop answers.

use gevrek_model::command::DispatchCommand;

#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    Trigger(String),
    Dispatch(DispatchCommand),
    DispatchBatch {
        commands: Vec<DispatchCommand>,
        stop_on_error: bool,
    },
    /// Made-up input for a test (`input_injection`). Only sent when the
    /// window manager was started to accept it.
    InjectInput(Vec<crate::input_injection::InjectedInput>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCommandResult {
    pub success: bool,
    pub skipped: bool,
    pub error: Option<String>,
}

pub fn run_dispatch_batch<T, F>(
    commands: &[T],
    stop_on_error: bool,
    mut execute: F,
) -> Vec<BatchCommandResult>
where
    F: FnMut(&T) -> Result<(), String>,
{
    let mut stopped = false;
    commands
        .iter()
        .map(|command| {
            if stopped {
                return BatchCommandResult {
                    success: false,
                    skipped: true,
                    error: Some("Skipped after an earlier batch command failed".to_string()),
                };
            }
            match execute(command) {
                Ok(()) => BatchCommandResult {
                    success: true,
                    skipped: false,
                    error: None,
                },
                Err(error) => {
                    stopped = stop_on_error;
                    BatchCommandResult {
                        success: false,
                        skipped: false,
                        error: Some(error),
                    }
                }
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCommandResult {
    Complete,
    Batch(Vec<BatchCommandResult>),
}

pub struct ManualCommand {
    pub command: RuntimeCommand,
    pub response: tokio::sync::oneshot::Sender<Result<RuntimeCommandResult, String>>,
}

impl ManualCommand {
    /// The IPC caller stops waiting after its reply timeout and drops the
    /// receiver; such a command must not run any more.
    pub fn caller_gave_up(&self) -> bool {
        self.response.is_closed()
    }
}

#[cfg(test)]
mod manual_command_tests {
    use super::*;
    use gevrek_model::command::DispatchCommand;

    #[test]
    fn a_command_whose_caller_timed_out_is_recognised() {
        let (response, receiver) = tokio::sync::oneshot::channel();
        let command = ManualCommand {
            command: RuntimeCommand::Dispatch(DispatchCommand::Retile),
            response,
        };
        assert!(!command.caller_gave_up());
        drop(receiver);
        assert!(command.caller_gave_up());
    }
}
