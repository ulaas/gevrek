//! Value, arithmetic, conversion, logging and loop nodes.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_value_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Math nodes
            NodeType::Add => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                self.set_output_value(node_id, 0, Value::Number(a + b));
            }

            NodeType::Subtract => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                self.set_output_value(node_id, 0, Value::Number(a - b));
            }

            NodeType::Multiply => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                self.set_output_value(node_id, 0, Value::Number(a * b));
            }

            NodeType::Divide => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);

                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(1.0);

                self.set_output_value(node_id, 0, Value::Number(a / b));
            }

            // Value nodes
            NodeType::NumberValue { value } => {
                self.set_output_value(node_id, 0, Value::Number(*value));
            }

            NodeType::StringValue { value } => {
                self.set_output_value(node_id, 0, Value::String(value.clone()));
            }

            NodeType::BoolValue { value } => {
                self.set_output_value(node_id, 0, Value::Bool(*value));
            }

            NodeType::ColorValue { r, g, b } => {
                self.set_output_value(node_id, 0, Value::Color(*r, *g, *b));
            }

            NodeType::BorderStyleValue { value } => {
                self.set_output_value(node_id, 0, Value::BorderStyle(*value));
            }

            // Conversion nodes
            NodeType::ToString => {
                // Convert Number or Bool to String
                if let Some(value) = self.get_input_value(node_id, 0) {
                    let string_value = match value {
                        Value::Number(n) => {
                            // Format numbers nicely (remove unnecessary decimals)
                            if n.fract() == 0.0 {
                                format!("{}", n as i64)
                            } else {
                                format!("{}", n)
                            }
                        }
                        Value::Bool(b) => format!("{}", b),
                        Value::String(s) => s, // Already a string, passthrough
                        _ => format!("{:?}", value), // Fallback for other types
                    };
                    self.set_output_value(node_id, 0, Value::String(string_value));
                }
            }

            NodeType::FromString => {
                // Convert String to Number (parse)
                if let Some(Value::String(s)) = self.get_input_value(node_id, 0) {
                    // Try to parse as number
                    if let Ok(n) = s.parse::<f64>() {
                        self.set_output_value(node_id, 0, Value::Number(n));
                        log::trace!("Parsed '{}' as number: {}", s, n);
                    } else {
                        log::warn!("Failed to parse '{}' as number, defaulting to 0.0", s);
                        self.set_output_value(node_id, 0, Value::Number(0.0));
                    }
                }
            }

            // Debug nodes
            NodeType::LogMessage { message, level } => {
                // Build final message: base message + optional text from input pin
                let mut final_message = message.clone();

                // Check if Text input pin (index 1) has a value
                if let Some(Value::String(text)) = self.get_input_value(node_id, 1) {
                    final_message.push_str(&text);
                }

                // Log to standard logging system based on level
                match level {
                    gevrek_model::graph::LogLevel::Debug => {
                        log::debug!("[Visual Log] {}", final_message)
                    }
                    gevrek_model::graph::LogLevel::Info => {
                        log::info!("[Visual Log] {}", final_message)
                    }
                    gevrek_model::graph::LogLevel::Warning => {
                        log::warn!("[Visual Log] {}", final_message)
                    }
                    gevrek_model::graph::LogLevel::Error => {
                        log::error!("[Visual Log] {}", final_message)
                    }
                }

                // Add to visual log
                if let Ok(mut logs) = self.log_entries.lock() {
                    logs.push(LogEntry {
                        timestamp: std::time::Instant::now(),
                        level: *level,
                        message: final_message.clone(),
                    });
                }

                // Broadcast to connected clients (editors) if callback is available
                if let Some(broadcast) = &self.log_broadcast {
                    let timestamp_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    broadcast(final_message, *level, timestamp_ms);
                }

                // Trigger next execution
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ForEach => {
                // Get list input value (index 1)
                let input_value = self.get_input_value(node_id, 1);
                log::trace!(
                    "🔄 ForEach (node {}): Input value is {:?}",
                    node_id,
                    input_value
                );

                // Handle WindowList
                if let Some(Value::WindowList(windows)) = input_value {
                    log::trace!(
                        "🔄 ForEach (node {}): Starting iteration over {} windows",
                        node_id,
                        windows.len()
                    );

                    // For each window in the list
                    for (index, window_handle) in windows.iter().enumerate() {
                        if self.nodes_spent() {
                            break;
                        }
                        log::trace!(
                            "🔄 ForEach (node {}): Iteration {}/{} - window {:?}",
                            node_id,
                            index + 1,
                            windows.len(),
                            window_handle
                        );

                        // Set current window and index outputs
                        self.set_output_value(node_id, 1, Value::WindowHandle(*window_handle));
                        self.set_output_value(node_id, 2, Value::Number(index as f64));

                        // CRITICAL: Create a fresh visited set for each iteration
                        // This allows the loop body nodes to execute again for each item
                        // We start with only the ForEach node itself to prevent re-entering the loop
                        let mut loop_visited = HashSet::new();
                        loop_visited.insert(node_id);

                        // Trigger loop body execution (output index 0)
                        // CRITICAL: Continue on error - don't let one bad window stop the entire loop
                        log::trace!(
                            "🔄 ForEach (node {}): Triggering loop body for window {:?}",
                            node_id,
                            window_handle
                        );
                        if let Err(e) = self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            &mut loop_visited,
                        ) {
                            log::warn!("🔄 ForEach iteration {} failed for window {:?}: {:?} - continuing with next window", index, window_handle, e);
                        }
                        log::trace!(
                            "🔄 ForEach (node {}): Completed iteration {} for window {:?}",
                            node_id,
                            index,
                            window_handle
                        );
                    }
                    log::trace!(
                        "🔄 ForEach (node {}): Completed all {} iterations",
                        node_id,
                        windows.len()
                    );

                    // After all windows processed, trigger "Completed" output (output index 3)
                    self.trigger_output_execution(
                        window_manager,
                        monitor_manager,
                        node_id,
                        3,
                        visited,
                    )?;
                }
                // Handle MonitorList
                else if let Some(Value::MonitorList(monitors)) = self.get_input_value(node_id, 1)
                {
                    log::trace!("ForEach: Iterating over {} monitors", monitors.len());

                    // For each monitor in the list
                    for (index, monitor_handle) in monitors.iter().enumerate() {
                        if self.nodes_spent() {
                            break;
                        }
                        // Set current monitor and index outputs
                        self.set_output_value(node_id, 1, Value::Monitor(*monitor_handle));
                        self.set_output_value(node_id, 2, Value::Number(index as f64));

                        // CRITICAL: Create a fresh visited set for each iteration
                        // This allows the loop body nodes to execute again for each item
                        // We start with only the ForEach node itself to prevent re-entering the loop
                        let mut loop_visited = HashSet::new();
                        loop_visited.insert(node_id);

                        // Trigger loop body execution (output index 0)
                        // CRITICAL: Continue on error - don't let one bad monitor stop the entire loop
                        if let Err(e) = self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            &mut loop_visited,
                        ) {
                            log::warn!("ForEach iteration {} failed for monitor {:?}: {:?} - continuing with next monitor", index, monitor_handle, e);
                        }
                    }

                    // After all monitors processed, trigger "Completed" output (output index 3)
                    self.trigger_output_execution(
                        window_manager,
                        monitor_manager,
                        node_id,
                        3,
                        visited,
                    )?;
                }
            }

            NodeType::ForLoop { start, end, step } => {
                // Get start, end, step from inputs or use node defaults
                let start_val = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n as i32)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(*start);

                let end_val = self
                    .get_input_value(node_id, 2)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n as i32)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(*end);

                let step_val = self
                    .get_input_value(node_id, 3)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n as i32)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(*step);

                // Ensure step is not zero to avoid infinite loop
                if step_val == 0 {
                    log::error!("ForLoop: Step cannot be zero, skipping loop");
                } else {
                    log::trace!(
                        "ForLoop: Iterating from {} to {} with step {}",
                        start_val,
                        end_val,
                        step_val
                    );

                    // Loop with the given range and step
                    let mut current = start_val;
                    let mut iteration_count = 0;
                    const MAX_ITERATIONS: usize = 10000; // Safety limit

                    while (step_val > 0 && current <= end_val)
                        || (step_val < 0 && current >= end_val)
                    {
                        if self.nodes_spent() {
                            break;
                        }
                        if iteration_count >= MAX_ITERATIONS {
                            log::error!(
                                "ForLoop: Maximum iteration limit reached ({}), stopping loop",
                                MAX_ITERATIONS
                            );
                            break;
                        }

                        // Set current index output (output index 1)
                        self.set_output_value(node_id, 1, Value::Number(current as f64));

                        // CRITICAL: Create a fresh visited set for each iteration
                        // This allows the loop body nodes to execute again for each iteration
                        // We start with only the ForLoop node itself to prevent re-entering the loop
                        let mut loop_visited = HashSet::new();
                        loop_visited.insert(node_id);

                        // Trigger loop body execution (output index 0)
                        // CRITICAL: Continue on error - don't let one bad iteration stop the entire loop
                        if let Err(e) = self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            &mut loop_visited,
                        ) {
                            log::warn!("ForLoop iteration {} failed: {:?} - continuing with next iteration", current, e);
                        }

                        iteration_count += 1;
                        // (Past the largest number there is, there is no
                        // next one: the loop is over.)
                        let Some(next) = current.checked_add(step_val) else {
                            break;
                        };
                        current = next;
                    }

                    log::trace!("ForLoop: Completed {} iterations", iteration_count);

                    // After loop completes, trigger "Completed" output (output index 2)
                    self.trigger_output_execution(
                        window_manager,
                        monitor_manager,
                        node_id,
                        2,
                        visited,
                    )?;
                }
            }

            NodeType::WindowListToSingle { index } => {
                if let Some(Value::WindowList(windows)) = self.get_input_value(node_id, 1) {
                    let idx = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as usize)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*index);

                    if idx < windows.len() {
                        self.set_output_value(node_id, 1, Value::WindowHandle(windows[idx]));
                        log::trace!("Extracted window at index {}", idx);
                    } else {
                        log::warn!(
                            "Index {} out of bounds (list has {} windows)",
                            idx,
                            windows.len()
                        );
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Other families are run by their own function.
            _ => {}
        }
        Ok(NodeDone::Normal)
    }
}
