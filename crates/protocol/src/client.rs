// gRPC client for the editor to communicate with the WM
use crate::proto::{self, window_manager_client::WindowManagerClient};
use anyhow::Result;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

/// Configuration for connection retry with exponential backoff
#[derive(Clone)]
pub struct RetryConfig {
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Maximum number of retry attempts (0 = infinite)
    pub max_attempts: u32,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_attempts: 10,
            backoff_multiplier: 2.0,
        }
    }
}

/// Deadline for calls that only read state.
const QUERY_DEADLINE: Duration = Duration::from_secs(5);
/// Deadline for calls the WM answers after its runtime acted (it waits up
/// to 10 s for the runtime itself).
const COMMAND_DEADLINE: Duration = Duration::from_secs(12);

/// A request with a deadline: a WM that stopped answering must not hang a
/// client (the editor polls from its UI loop).
fn with_deadline<T>(message: T, deadline: Duration) -> Request<T> {
    let mut request = Request::new(message);
    request.set_timeout(deadline);
    request
}

/// Wrapper around the gRPC client with convenience methods. Cloning is
/// cheap (the clones share one connection), so concurrent tasks can each
/// hold their own instead of waiting on a lock.
#[derive(Clone)]
pub struct WMClient {
    client: WindowManagerClient<Channel>,
    address: String,
}

/// True when the authority names a port that is not a valid `u16`.
/// `Uri::port()` returns `None` for such a port, just as for a missing one.
fn has_invalid_port(uri: &tonic::codegen::http::Uri) -> bool {
    let Some(authority) = uri.authority() else {
        return false;
    };
    let host_and_port = authority.as_str().rsplit('@').next().unwrap_or_default();
    let port = match host_and_port.rsplit_once(':') {
        Some((host, port)) if !port.ends_with(']') && !host.ends_with(':') => port,
        _ => return false,
    };
    !port.is_empty() && port.parse::<u16>().is_err()
}

impl WMClient {
    async fn connect_channel(addr: &str) -> Result<Channel> {
        let addr = match crate::Endpoint::parse(addr) {
            crate::Endpoint::Local(instance) => {
                return crate::endpoint::connect_local(&instance).await;
            }
            crate::Endpoint::Tcp(url) => url,
        };
        let addr = addr.as_str();
        let endpoint =
            Endpoint::from_shared(addr.to_owned())?.connect_timeout(Duration::from_secs(2));
        // The URI parser accepts any digits as a port, and the connector then
        // silently falls back to the scheme default (80) for an out-of-range
        // one, so a typo would connect to whatever listens there.
        if has_invalid_port(endpoint.uri()) {
            anyhow::bail!("invalid port in WM address {addr:?}");
        }
        Ok(endpoint.connect().await?)
    }

    /// Connect to the WM server (single attempt)
    pub async fn connect(addr: String) -> Result<Self> {
        let client = WindowManagerClient::new(Self::connect_channel(&addr).await?);
        Ok(Self {
            client,
            address: addr,
        })
    }

    /// Connect to the WM server with exponential backoff retry
    ///
    /// # Arguments
    /// * `addr` - The server address (e.g., "http://127.0.0.1:57651")
    /// * `config` - Retry configuration (use RetryConfig::default() for sensible defaults)
    ///
    /// # Returns
    /// * `Ok(WMClient)` - Connected client
    /// * `Err` - If all retry attempts fail
    pub async fn connect_with_retry(addr: String, config: RetryConfig) -> Result<Self> {
        let mut attempts = 0u32;
        let mut current_delay = config.initial_delay;

        loop {
            attempts += 1;
            log::debug!("Connection attempt {} to {}", attempts, addr);

            match Self::connect_channel(&addr).await {
                Ok(channel) => {
                    if attempts > 1 {
                        log::info!("Successfully connected after {} attempts", attempts);
                    }
                    return Ok(Self {
                        client: WindowManagerClient::new(channel),
                        address: addr,
                    });
                }
                Err(e) => {
                    // Check if we've exhausted retry attempts
                    if config.max_attempts > 0 && attempts >= config.max_attempts {
                        log::error!("Failed to connect after {} attempts: {}", attempts, e);
                        return Err(anyhow::anyhow!(
                            "Connection failed after {} attempts: {}",
                            attempts,
                            e
                        ));
                    }

                    log::warn!(
                        "Connection attempt {} failed: {}. Retrying in {:?}...",
                        attempts,
                        e,
                        current_delay
                    );

                    // Sleep before next retry
                    tokio::time::sleep(current_delay).await;

                    // Calculate next delay with exponential backoff
                    current_delay = Duration::from_millis(
                        ((current_delay.as_millis() as f64) * config.backoff_multiplier) as u64,
                    );
                    // Cap at max delay
                    if current_delay > config.max_delay {
                        current_delay = config.max_delay;
                    }
                }
            }
        }
    }

    /// Reconnect to the WM server (useful after connection drops)
    pub async fn reconnect(&mut self) -> Result<()> {
        log::info!("Attempting to reconnect to {}", self.address);
        self.client = WindowManagerClient::new(Self::connect_channel(&self.address).await?);
        log::info!("Reconnected successfully");
        Ok(())
    }

    /// Reconnect with exponential backoff retry
    pub async fn reconnect_with_retry(&mut self, config: RetryConfig) -> Result<()> {
        let new_client = Self::connect_with_retry(self.address.clone(), config).await?;
        self.client = new_client.client;
        Ok(())
    }

    /// Get the server address this client is connected to
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Push a new node graph to the WM
    pub async fn push_graph(
        &mut self,
        graph: proto::NodeGraph,
    ) -> Result<proto::PushGraphResponse> {
        let response = self
            .client
            .push_graph(with_deadline(graph, COMMAND_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Fetch the complete versioned desktop configuration.
    pub async fn get_configuration(&mut self) -> Result<proto::ConfigurationDocument> {
        let response = self
            .client
            .get_configuration(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Validate, persist, and atomically apply the complete desktop configuration.
    pub async fn update_configuration(
        &mut self,
        document: proto::ConfigurationDocument,
    ) -> Result<proto::UpdateConfigurationResponse> {
        let mut request = Request::new(document);
        request.set_timeout(Duration::from_secs(5));
        let response = self.client.update_configuration(request).await?;
        Ok(response.into_inner())
    }

    /// Pause execution
    pub async fn pause_execution(&mut self) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .pause_execution(with_deadline(proto::Empty {}, COMMAND_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Resume execution
    pub async fn resume_execution(&mut self) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .resume_execution(with_deadline(proto::Empty {}, COMMAND_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Trigger a manual event
    pub async fn trigger_manual_event(&mut self, node_id: String) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .trigger_manual_event(with_deadline(
                proto::TriggerRequest { node_id },
                COMMAND_DEADLINE,
            ))
            .await?;
        Ok(response.into_inner())
    }

    /// Execute a typed window-manager command.
    pub async fn dispatch(
        &mut self,
        request: proto::DispatchRequest,
    ) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .dispatch(with_deadline(request, COMMAND_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Run a command given as the JSON the configuration uses for it. Works
    /// for every command, also ones added after this client was built.
    pub async fn dispatch_command(
        &mut self,
        json: impl Into<String>,
    ) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .dispatch_command(with_deadline(
                proto::CommandDocument { json: json.into() },
                COMMAND_DEADLINE,
            ))
            .await?;
        Ok(response.into_inner())
    }

    /// Feed made-up input (a JSON array of steps) to a window manager that
    /// was started to accept it.
    pub async fn inject_input(&mut self, json: impl Into<String>) -> Result<proto::StatusResponse> {
        let mut request = Request::new(proto::CommandDocument { json: json.into() });
        request.set_timeout(Duration::from_secs(25));
        Ok(self.client.inject_input(request).await?.into_inner())
    }

    /// Execute a validated, uninterrupted batch of typed WM commands.
    pub async fn dispatch_batch(
        &mut self,
        requests: Vec<proto::DispatchRequest>,
        stop_on_error: bool,
    ) -> Result<proto::DispatchBatchResponse> {
        let response = self
            .client
            .dispatch_batch(with_deadline(
                proto::DispatchBatchRequest {
                    requests,
                    stop_on_error,
                },
                COMMAND_DEADLINE,
            ))
            .await?;
        Ok(response.into_inner())
    }

    /// Query WM state
    pub async fn query_state(&mut self) -> Result<proto::WmState> {
        let response = self
            .client
            .query_state(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Subscribe to execution events
    pub async fn subscribe_to_events(&mut self) -> Result<tonic::Streaming<proto::ExecutionEvent>> {
        let response = self
            .client
            .subscribe_to_events(Request::new(proto::Empty {}))
            .await?;
        Ok(response.into_inner())
    }

    /// Health check
    pub async fn health_check(&mut self) -> Result<proto::HealthStatus> {
        let response = self
            .client
            .health_check(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Discover the running server's protocol and feature surface.
    pub async fn get_capabilities(&mut self) -> Result<proto::CapabilitiesResponse> {
        let response = self
            .client
            .get_capabilities(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Get node definitions from WM server
    pub async fn get_node_definitions(&mut self) -> Result<proto::GetNodeDefinitionsResponse> {
        let response = self
            .client
            .get_node_definitions(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Get the current active node graph from WM server
    pub async fn get_current_graph(&mut self) -> Result<proto::NodeGraph> {
        let response = self
            .client
            .get_current_graph(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Update the working (collaborative) graph
    pub async fn update_working_graph(
        &mut self,
        graph: proto::NodeGraph,
    ) -> Result<proto::StatusResponse> {
        let response = self
            .client
            .update_working_graph(with_deadline(graph, COMMAND_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Get the current working graph (collaborative draft)
    pub async fn get_working_graph(&mut self) -> Result<proto::NodeGraph> {
        let response = self
            .client
            .get_working_graph(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Get overlay/debug data for display
    pub async fn get_overlay_data(&mut self) -> Result<proto::OverlayDataResponse> {
        let response = self
            .client
            .get_overlay_data(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Request the WM server to shutdown gracefully
    pub async fn shutdown(
        &mut self,
        save_config: bool,
        delay_ms: u32,
    ) -> Result<proto::StatusResponse> {
        let mut request = Request::new(proto::ShutdownRequest {
            save_config,
            delay_ms,
        });
        request.set_timeout(Duration::from_secs(2));
        let response = self.client.shutdown(request).await?;
        Ok(response.into_inner())
    }

    /// Become the shell that runs the zones: what `reports` gives is told
    /// to the window manager, and what is asked of the zones comes back
    /// as the stream. The call lasts for as long as the zones run, so it
    /// has no deadline.
    pub async fn attach_zones(
        &mut self,
        reports: impl tokio_stream::Stream<Item = proto::ZonesReport> + Send + 'static,
    ) -> Result<tonic::Streaming<proto::ZonesRequest>> {
        let response = self.client.attach_zones(Request::new(reports)).await?;
        Ok(response.into_inner())
    }

    /// How the zones are, as the shell last reported it.
    pub async fn get_zones_state(&mut self) -> Result<proto::ZonesStateResponse> {
        let response = self
            .client
            .get_zones_state(with_deadline(proto::Empty {}, QUERY_DEADLINE))
            .await?;
        Ok(response.into_inner())
    }

    /// Ask something of the zones (a zones command as JSON) and wait for
    /// the shell's answer: for `timeout_ms`, or the window manager's own
    /// time where that is 0.
    pub async fn send_zones_command(
        &mut self,
        json: impl Into<String>,
        version: u32,
        timeout_ms: u32,
    ) -> Result<proto::ZonesCommandReply> {
        // The window manager answers when its own wait is over; this one
        // is a little longer, so that its answer is the one shown.
        let waited = Duration::from_millis(u64::from(timeout_ms.min(30_000)));
        let response = self
            .client
            .send_zones_command(with_deadline(
                proto::ZonesCommandDocument {
                    json: json.into(),
                    version,
                    timeout_ms,
                },
                COMMAND_DEADLINE.max(waited + Duration::from_secs(2)),
            ))
            .await?;
        Ok(response.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::has_invalid_port;

    fn invalid(addr: &str) -> bool {
        has_invalid_port(&addr.parse().unwrap())
    }

    #[test]
    fn out_of_range_ports_are_rejected() {
        assert!(invalid("http://127.0.0.1:99999"));
        assert!(invalid("http://[::1]:70000"));
        assert!(!invalid("http://127.0.0.1:57651"));
        assert!(!invalid("http://[::1]:57651"));
        assert!(!invalid("http://[::1]"));
        assert!(!invalid("http://localhost"));
    }
}
