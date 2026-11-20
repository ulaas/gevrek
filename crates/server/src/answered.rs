//! What the server is asked and how it answers, written to the log.
//!
//! The server the protocol is served from is [`Logged`]: it hands every
//! call to the [`WMServer`] and says, at debug, which call came and what
//! came of it. A call that is asked many times a second (the state the
//! overlay and the shell poll, the health that `gevrek start` waits on) is
//! said at trace; that it fails is said at debug once, when it begins,
//! and once when it is answered again.
//!
//! What is asked is said by its kind, never by its whole payload: no
//! configuration, no input that is made up, no command's arguments.

use crate::WMServer;
use gevrek_protocol::window_manager_server::WindowManager as WMService;
use gevrek_protocol::*;
use log::Level;
use std::sync::atomic::{AtomicBool, Ordering};
use tonic::{Request, Response, Status};

/// The server as it is served: [`WMServer`], with each call logged.
pub(crate) struct Logged {
    server: WMServer,
    overlay: Polled,
    state: Polled,
    health: Polled,
    zones_state: Polled,
}

impl Logged {
    pub(crate) fn new(server: WMServer) -> Self {
        Self {
            server,
            overlay: Polled::default(),
            state: Polled::default(),
            health: Polled::default(),
            zones_state: Polled::default(),
        }
    }
}

/// Whether a call that is polled fails now, so that it is said once.
#[derive(Default)]
struct Polled {
    failing: AtomicBool,
}

/// What an answer says of itself where the call went through but what was
/// asked was not done.
trait Refusal {
    fn refusal(&self) -> Option<String> {
        None
    }
}

fn refused(success: bool, error: &Option<String>) -> Option<String> {
    (!success).then(|| {
        error
            .clone()
            .unwrap_or_else(|| "no reason given".to_string())
    })
}

impl Refusal for StatusResponse {
    fn refusal(&self) -> Option<String> {
        refused(self.success, &self.error)
    }
}

impl Refusal for PushGraphResponse {
    fn refusal(&self) -> Option<String> {
        refused(self.success, &self.error)
    }
}

impl Refusal for UpdateConfigurationResponse {
    fn refusal(&self) -> Option<String> {
        refused(self.success, &self.error)
    }
}

impl Refusal for ZonesCommandReply {
    fn refusal(&self) -> Option<String> {
        refused(self.success, &self.error)
    }
}

impl Refusal for DispatchBatchResponse {
    fn refusal(&self) -> Option<String> {
        if self.success {
            return None;
        }
        let failed = self.results.iter().filter(|result| !result.success).count();
        let first = self
            .results
            .iter()
            .find_map(|result| result.error.clone())
            .unwrap_or_default();
        Some(format!(
            "{failed} of {} commands not done (the first: {first})",
            self.results.len()
        ))
    }
}

impl Refusal for HealthStatus {
    fn refusal(&self) -> Option<String> {
        (!self.healthy).then(|| {
            format!(
                "not healthy: {}",
                self.message.as_deref().unwrap_or("no reason given")
            )
        })
    }
}

impl Refusal for ConfigurationDocument {}
impl Refusal for WmState {}
impl Refusal for CapabilitiesResponse {}
impl Refusal for GetNodeDefinitionsResponse {}
impl Refusal for NodeGraph {}
impl Refusal for OverlayDataResponse {}
impl Refusal for ZonesStateResponse {}
impl<T> Refusal for tokio_stream::wrappers::ReceiverStream<T> {}

/// Say how `rpc` was answered, at `level`.
fn answered<T: Refusal>(rpc: &str, level: Level, result: &Result<Response<T>, Status>) {
    match result {
        Ok(response) => match response.get_ref().refusal() {
            None => log::log!(level, "{rpc} was answered"),
            Some(why) => log::log!(level, "{rpc} was answered, but not done: {why}"),
        },
        Err(status) => log::log!(
            level,
            "{rpc} was refused ({:?}): {}",
            status.code(),
            status.message()
        ),
    }
}

/// Say how a polled `rpc` was answered: each time at trace, and at debug
/// when it begins to fail and when it is answered again.
fn answered_polled<T: Refusal>(rpc: &str, polled: &Polled, result: &Result<Response<T>, Status>) {
    answered(rpc, Level::Trace, result);
    match result {
        Err(status) => {
            if !polled.failing.swap(true, Ordering::Relaxed) {
                log::debug!(
                    "{rpc} fails from now on, until it is answered again ({:?}): {}",
                    status.code(),
                    status.message()
                );
            }
        }
        Ok(_) => {
            if polled.failing.swap(false, Ordering::Relaxed) {
                log::debug!("{rpc} is answered again");
            }
        }
    }
}

/// The name of the command in a command's JSON (`{"command": "quit", ..}`),
/// without its arguments.
fn command_name(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("command")
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "a command that is not read".to_string())
}

#[tonic::async_trait]
impl WMService for Logged {
    async fn push_graph(
        &self,
        request: Request<NodeGraph>,
    ) -> Result<Response<PushGraphResponse>, Status> {
        let graph = request.get_ref();
        log::debug!(
            "PushGraph was asked by editor {:?}: {} nodes, {} connections, from revision {}",
            graph.editor_id,
            graph.node_count,
            graph.connection_count,
            graph.base_revision
        );
        let result = self.server.push_graph(request).await;
        answered("PushGraph", Level::Debug, &result);
        result
    }

    async fn get_configuration(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<ConfigurationDocument>, Status> {
        log::debug!("GetConfiguration was asked");
        let result = self.server.get_configuration(request).await;
        match &result {
            Ok(document) => log::debug!(
                "GetConfiguration was answered with revision {}",
                document.get_ref().revision
            ),
            Err(_) => answered("GetConfiguration", Level::Debug, &result),
        }
        result
    }

    async fn update_configuration(
        &self,
        request: Request<ConfigurationDocument>,
    ) -> Result<Response<UpdateConfigurationResponse>, Status> {
        let document = request.get_ref();
        log::debug!(
            "UpdateConfiguration was asked by client {:?}: {} bytes, from revision {}",
            document.client_id,
            document.json.len(),
            document.base_revision
        );
        let result = self.server.update_configuration(request).await;
        match &result {
            Ok(response) if response.get_ref().success => log::debug!(
                "UpdateConfiguration was answered: applied as revision {}",
                response.get_ref().revision
            ),
            _ => answered("UpdateConfiguration", Level::Debug, &result),
        }
        result
    }

    async fn pause_execution(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::debug!("PauseExecution was asked");
        let result = self.server.pause_execution(request).await;
        answered("PauseExecution", Level::Debug, &result);
        result
    }

    async fn resume_execution(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::debug!("ResumeExecution was asked");
        let result = self.server.resume_execution(request).await;
        answered("ResumeExecution", Level::Debug, &result);
        result
    }

    async fn trigger_manual_event(
        &self,
        request: Request<TriggerRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::debug!(
            "TriggerManualEvent was asked for node {}",
            request.get_ref().node_id
        );
        let result = self.server.trigger_manual_event(request).await;
        answered("TriggerManualEvent", Level::Debug, &result);
        result
    }

    async fn dispatch(
        &self,
        request: Request<DispatchRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let command = DispatchType::try_from(request.get_ref().command)
            .map(|command| command.as_str_name())
            .unwrap_or("an unknown dispatcher");
        log::debug!("Dispatch was asked: {command}");
        let result = self.server.dispatch(request).await;
        answered("Dispatch", Level::Debug, &result);
        result
    }

    async fn inject_input(
        &self,
        request: Request<CommandDocument>,
    ) -> Result<Response<StatusResponse>, Status> {
        // What the input is, is never said: it is keys and text.
        log::debug!("InjectInput was asked");
        let result = self.server.inject_input(request).await;
        answered("InjectInput", Level::Debug, &result);
        result
    }

    async fn dispatch_command(
        &self,
        request: Request<CommandDocument>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::debug!(
            "DispatchCommand was asked: {}",
            command_name(&request.get_ref().json)
        );
        let result = self.server.dispatch_command(request).await;
        answered("DispatchCommand", Level::Debug, &result);
        result
    }

    async fn dispatch_batch(
        &self,
        request: Request<DispatchBatchRequest>,
    ) -> Result<Response<DispatchBatchResponse>, Status> {
        log::debug!(
            "DispatchBatch was asked: {} commands (stop on an error: {})",
            request.get_ref().requests.len(),
            request.get_ref().stop_on_error
        );
        let result = self.server.dispatch_batch(request).await;
        answered("DispatchBatch", Level::Debug, &result);
        result
    }

    async fn query_state(&self, request: Request<Empty>) -> Result<Response<WmState>, Status> {
        log::trace!("QueryState was asked");
        let result = self.server.query_state(request).await;
        answered_polled("QueryState", &self.state, &result);
        result
    }

    type SubscribeToEventsStream = <WMServer as WMService>::SubscribeToEventsStream;

    async fn subscribe_to_events(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<Self::SubscribeToEventsStream>, Status> {
        log::debug!("SubscribeToEvents was asked");
        let result = self.server.subscribe_to_events(request).await;
        answered("SubscribeToEvents", Level::Debug, &result);
        result
    }

    async fn health_check(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<HealthStatus>, Status> {
        log::trace!("HealthCheck was asked");
        let result = self.server.health_check(request).await;
        answered_polled("HealthCheck", &self.health, &result);
        result
    }

    async fn get_capabilities(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        log::debug!("GetCapabilities was asked");
        let result = self.server.get_capabilities(request).await;
        answered("GetCapabilities", Level::Debug, &result);
        result
    }

    async fn get_node_definitions(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<GetNodeDefinitionsResponse>, Status> {
        log::debug!("GetNodeDefinitions was asked");
        let result = self.server.get_node_definitions(request).await;
        answered("GetNodeDefinitions", Level::Debug, &result);
        result
    }

    async fn get_current_graph(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<NodeGraph>, Status> {
        log::debug!("GetCurrentGraph was asked");
        let result = self.server.get_current_graph(request).await;
        answered("GetCurrentGraph", Level::Debug, &result);
        result
    }

    async fn update_working_graph(
        &self,
        request: Request<NodeGraph>,
    ) -> Result<Response<StatusResponse>, Status> {
        // Said by the server itself, with the editor and the counts.
        let result = self.server.update_working_graph(request).await;
        answered("UpdateWorkingGraph", Level::Debug, &result);
        result
    }

    async fn get_working_graph(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<NodeGraph>, Status> {
        log::debug!("GetWorkingGraph was asked");
        let result = self.server.get_working_graph(request).await;
        answered("GetWorkingGraph", Level::Debug, &result);
        result
    }

    async fn get_overlay_data(
        &self,
        request: Request<Empty>,
    ) -> Result<Response<OverlayDataResponse>, Status> {
        log::trace!("GetOverlayData was asked");
        let result = self.server.get_overlay_data(request).await;
        answered_polled("GetOverlayData", &self.overlay, &result);
        result
    }

    async fn shutdown(
        &self,
        request: Request<ShutdownRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        // Said by the server itself, at info.
        let result = self.server.shutdown(request).await;
        answered("Shutdown", Level::Debug, &result);
        result
    }

    type AttachZonesStream = <WMServer as WMService>::AttachZonesStream;

    async fn attach_zones(
        &self,
        request: Request<tonic::Streaming<ZonesReport>>,
    ) -> Result<Response<Self::AttachZonesStream>, Status> {
        log::debug!("AttachZones was asked: a shell offers to run the zones");
        let result = self.server.attach_zones(request).await;
        answered("AttachZones", Level::Debug, &result);
        result
    }

    async fn get_zones_state(
        &self,
        request: Request<ZonesStateRequest>,
    ) -> Result<Response<ZonesStateResponse>, Status> {
        log::trace!("GetZonesState was asked");
        let result = self.server.get_zones_state(request).await;
        answered_polled("GetZonesState", &self.zones_state, &result);
        result
    }

    async fn send_zones_command(
        &self,
        request: Request<ZonesCommandDocument>,
    ) -> Result<Response<ZonesCommandReply>, Status> {
        log::debug!(
            "SendZonesCommand was asked: {}",
            command_name(&request.get_ref().json)
        );
        let result = self.server.send_zones_command(request).await;
        answered("SendZonesCommand", Level::Debug, &result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_named_without_its_arguments() {
        assert_eq!(
            command_name(r#"{"command": "exec", "argument": "secret"}"#),
            "exec"
        );
        assert_eq!(command_name("not json"), "a command that is not read");
    }

    #[test]
    fn an_answer_that_was_not_done_says_why() {
        let refused = StatusResponse {
            success: false,
            error: Some("Execution is paused".to_string()),
        };
        assert_eq!(refused.refusal().as_deref(), Some("Execution is paused"));
        let done = StatusResponse {
            success: true,
            error: None,
        };
        assert_eq!(done.refusal(), None);
    }
}
