use gevrek_protocol::*;

/// Central registry of all available node types.
///
/// The model is the source of truth: every node type in its catalog has a
/// definition here, with the pins the model gives that node, its display
/// name and its catalog group. The hand-written table below only adds the
/// help texts, property descriptions and the legacy enum value.
pub struct NodeRegistry {
    definitions: Vec<NodeDefinition>,
}

/// The serde name of a node type (`"OnKeyPress"`), used as its `type_id`.
fn type_id_of(node_type: &gevrek_model::graph::NodeType) -> String {
    match serde_json::to_value(node_type) {
        Ok(serde_json::Value::String(name)) => name,
        Ok(serde_json::Value::Object(map)) => map.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Help texts for the node types the hand-written table does not cover.
fn description_of(type_id: &str) -> &'static str {
    match type_id {
        "FocusNextWindow" => "Focus the next window of the focused workspace",
        "FocusPreviousWindow" => "Focus the previous window of the focused workspace",
        "CycleFocusNext" => "Focus the next window in most-recently-used order (like Alt+Tab)",
        "CycleFocusPrevious" => "Focus the previous window in most-recently-used order",
        "SwapWithNext" => "Swap the focused window with the next one in the layout",
        "SwapWithPrevious" => "Swap the focused window with the previous one in the layout",
        "ToggleFloating" => "Switch the focused window between tiled and floating",
        "ToggleExposeMode" => "Show all windows of the workspace in an overview, or leave it",
        "CycleNextLayout" => "Switch the focused workspace to the next layout",
        "CyclePreviousLayout" => "Switch the focused workspace to the previous layout",
        "RotateLayout" => "Rotate the layout of the focused workspace by a quarter turn",
        "Retile" => "Set the layout of the focused workspace and lay its windows out again",
        "SetLayoutRatio" => "Set the share of the workspace the master area takes",
        "IncrementLayoutRatio" => "Make the master area larger by the step",
        "DecrementLayoutRatio" => "Make the master area smaller by the step",
        "SetMasterCount" => "Set how many windows the master area holds",
        "IncrementMasterCount" => "Put one more window into the master area",
        "DecrementMasterCount" => "Take one window out of the master area",
        "SetLayoutGap" => "Set the padding around the focused workspace, in pixels",
        "GetWorkspace" => "Output the workspace index set on this node",
        "MoveToNextWorkspace" => "Move the focused window to the next workspace",
        "MoveToPreviousWorkspace" => "Move the focused window to the previous workspace",
        "ToggleScratchpad" => {
            "Show or hide a scratchpad window; starts its program when no window matches"
        }
        "ShowScratchpad" => "Show the scratchpad with this id",
        "HideScratchpad" => "Hide the scratchpad with this id",
        "HideAllScratchpads" => "Hide every scratchpad that is showing",
        "CloakWindow" => "Hide a window without minimizing it, or show it again (Windows only)",
        "SetNativeBorderColor" => "Set the colour of the border Windows draws (Windows 11 only)",
        "SetCaptionColor" => "Set the title bar colour of a window (Windows 11 only)",
        "SetSystemBackdrop" => {
            "Set the backdrop material of a window, such as Mica (Windows 11 only)"
        }
        "SetWindowCorner" => "Set how the corners of a window are rounded (Windows 11 only)",
        "ApplySystemFilters" => {
            "Ignore system windows such as the taskbar, shell windows and tooltips"
        }
        "ForLoop" => "Run the loop body for each number from start to end, by step",
        "ToString" => "Convert a number or a true/false value to text",
        "FromString" => "Read text as a number (0 when it is not a number)",
        _ => "",
    }
}

fn pin_type_to_proto(pin_type: &gevrek_model::graph::PinType) -> PinType {
    use gevrek_model::graph::PinType as Model;
    match pin_type {
        Model::Execution => PinType::Execution,
        Model::WindowHandle => PinType::WindowHandle,
        Model::WindowList => PinType::WindowList,
        Model::Number => PinType::Number,
        Model::String => PinType::String,
        Model::Bool => PinType::Bool,
        Model::Color => PinType::Color,
        Model::BorderStyle => PinType::BorderStyle,
        Model::WindowRule => PinType::WindowRule,
        Model::MatchKind => PinType::MatchKind,
        Model::MatchStrategy => PinType::MatchStrategy,
        Model::Monitor => PinType::Monitor,
        Model::MonitorList => PinType::MonitorList,
        Model::Layout => PinType::Layout,
        Model::Rule => PinType::Rule,
        Model::RuleList => PinType::RuleList,
        Model::List => PinType::List,
        Model::Item => PinType::Item,
    }
}

/// The model's pins as definitions; a help text is taken from the
/// hand-written pin of the same name, if there is one.
fn pin_definitions(
    pins: &[gevrek_model::graph::Pin],
    described: Option<&[PinDefinition]>,
) -> Vec<PinDefinition> {
    pins.iter()
        .map(|pin| {
            let known = described.and_then(|pins| pins.iter().find(|known| known.name == pin.name));
            PinDefinition {
                name: pin.name.clone(),
                r#type: pin_type_to_proto(&pin.pin_type) as i32,
                required: known.is_some_and(|known| known.required),
                default_value: known.and_then(|known| known.default_value.clone()),
                description: known
                    .map(|known| known.description.clone())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

impl NodeRegistry {
    pub fn new() -> Self {
        let described = Self::hand_written();
        let mut definitions = Vec::new();
        for (group, node_types) in gevrek_model::graph::node_catalog() {
            for node_type in node_types {
                let type_id = type_id_of(&node_type);
                let known = described
                    .iter()
                    .find(|definition| definition.type_id == type_id);
                let mut graph = gevrek_model::graph::NodeGraph::new();
                let id = graph.add_node(node_type.clone(), [0.0, 0.0]);
                let node = &graph.nodes[&id];
                definitions.push(NodeDefinition {
                    type_id: type_id.clone(),
                    display_name: node_type.display_name().to_string(),
                    category: group.to_string(),
                    description: known
                        .map(|known| known.description.clone())
                        .unwrap_or_else(|| description_of(&type_id).to_string()),
                    inputs: pin_definitions(&node.inputs, known.map(|known| &known.inputs[..])),
                    outputs: pin_definitions(&node.outputs, known.map(|known| &known.outputs[..])),
                    properties: known
                        .map(|known| known.properties.clone())
                        .unwrap_or_default(),
                    legacy_type: gevrek_protocol::conversions::node_type_to_proto(&node_type)
                        as i32,
                });
            }
        }
        Self { definitions }
    }

    /// Help texts and property descriptions, written by hand for the node
    /// types that have them. Pins and names listed here are not used: they
    /// come from the model.
    // One push per node type, in sections: a long list that reads better
    // this way than as one ec![].
    #[allow(clippy::vec_init_then_push)]
    fn hand_written() -> Vec<NodeDefinition> {
        let mut definitions = Vec::new();

        // ====================================================================
        // Event Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "OnStartup".to_string(),
            display_name: "On Startup".to_string(),
            category: "Events".to_string(),
            description: "Triggered once when the window manager starts".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Event".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Execution flow".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::OnStartup as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnStartupCompleted".to_string(),
            display_name: "On Startup Completed".to_string(),
            category: "Events".to_string(),
            description: "Triggered after all OnStartup nodes have finished executing".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Event".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Execution flow".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::OnStartupCompleted as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnWindowCreated".to_string(),
            display_name: "On Window Created".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a new window is created".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "The created window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::OnWindowCreated as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnWindowDestroyed".to_string(),
            display_name: "On Window Destroyed".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a window is closed".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "The destroyed window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::OnWindowDestroyed as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnWindowFocused".to_string(),
            display_name: "On Window Focused".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a window gains focus".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "The focused window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::OnWindowFocused as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnWindowMoved".to_string(),
            display_name: "On Window Moved".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a window is moved".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "The moved window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::OnWindowMoved as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnWindowResized".to_string(),
            display_name: "On Window Resized".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a window is resized".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "The resized window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::OnWindowResized as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "OnKeyPress".to_string(),
            display_name: "On Key Press".to_string(),
            category: "Events".to_string(),
            description: "Triggered when a keyboard shortcut is pressed".to_string(),
            inputs: vec![],
            outputs: vec![
                PinDefinition {
                    name: "Event".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execution flow".to_string(),
                },
                PinDefinition {
                    name: "Key".to_string(),
                    r#type: PinType::String as i32,
                    required: false,
                    default_value: None,
                    description: "Key that was pressed".to_string(),
                },
                PinDefinition {
                    name: "Ctrl".to_string(),
                    r#type: PinType::Bool as i32,
                    required: false,
                    default_value: None,
                    description: "Ctrl modifier state".to_string(),
                },
                PinDefinition {
                    name: "Alt".to_string(),
                    r#type: PinType::Bool as i32,
                    required: false,
                    default_value: None,
                    description: "Alt modifier state".to_string(),
                },
                PinDefinition {
                    name: "Shift".to_string(),
                    r#type: PinType::Bool as i32,
                    required: false,
                    default_value: None,
                    description: "Shift modifier state".to_string(),
                },
                PinDefinition {
                    name: "Win".to_string(),
                    r#type: PinType::Bool as i32,
                    required: false,
                    default_value: None,
                    description: "Win modifier state".to_string(),
                },
            ],
            properties: vec![
                PropertyDefinition {
                    key: "key".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Key Combination".to_string(),
                    default_value: Some("Space".to_string()),
                    description: "Key to listen for".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "ctrl".to_string(),
                    r#type: PropertyType::BoolProperty as i32,
                    display_name: "Ctrl".to_string(),
                    default_value: Some("false".to_string()),
                    description: "Require Ctrl modifier".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "alt".to_string(),
                    r#type: PropertyType::BoolProperty as i32,
                    display_name: "Alt".to_string(),
                    default_value: Some("false".to_string()),
                    description: "Require Alt modifier".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "shift".to_string(),
                    r#type: PropertyType::BoolProperty as i32,
                    display_name: "Shift".to_string(),
                    default_value: Some("false".to_string()),
                    description: "Require Shift modifier".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "win".to_string(),
                    r#type: PropertyType::BoolProperty as i32,
                    display_name: "Win".to_string(),
                    default_value: Some("false".to_string()),
                    description: "Require Win modifier".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::OnKeyPress as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "ManualTrigger".to_string(),
            display_name: "Manual Trigger".to_string(),
            category: "Events".to_string(),
            description: "Manually trigger execution for testing".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Event".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Execution flow".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::ManualTrigger as i32,
        });

        // ====================================================================
        // Action Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "LaunchProcess".to_string(),
            display_name: "Launch Process".to_string(),
            category: "Actions".to_string(),
            description: "Launch an external program with structured arguments and options"
                .to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "program".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Program".to_string(),
                    default_value: Some("notepad.exe".to_string()),
                    description: "Executable name or path".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "args".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Arguments".to_string(),
                    default_value: Some("[]".to_string()),
                    description: "JSON array of arguments".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "working_directory".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Working Directory".to_string(),
                    default_value: Some(String::new()),
                    description: "Optional working directory".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "create_no_window".to_string(),
                    r#type: PropertyType::BoolProperty as i32,
                    display_name: "Hide Console Window".to_string(),
                    default_value: Some("false".to_string()),
                    description: "Do not create a console window".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::LaunchProcess as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "SetWindowArrangement".to_string(),
            display_name: "Set Window Arrangement".to_string(),
            category: "Actions".to_string(),
            description: "Arrange window in workspace (tiled, floating, etc.)".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to arrange".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "mode".to_string(),
                r#type: PropertyType::EnumProperty as i32,
                display_name: "Mode".to_string(),
                default_value: Some("Auto".to_string()),
                description: "Arrangement mode".to_string(),
                enum_values: vec![
                    "Auto".to_string(),
                    "Tiled".to_string(),
                    "Floating".to_string(),
                ],
            }],
            legacy_type: NodeType::SetWindowArrangement as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "MoveWindow".to_string(),
            display_name: "Move Window".to_string(),
            category: "Actions".to_string(),
            description: "Move a window to a specific position".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to move".to_string(),
                },
                PinDefinition {
                    name: "X".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "X position".to_string(),
                },
                PinDefinition {
                    name: "Y".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "Y position".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::MoveWindow as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "CloseWindow".to_string(),
            display_name: "Close Window".to_string(),
            category: "Actions".to_string(),
            description: "Close a window".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to close".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::CloseWindow as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "ResizeWindow".to_string(),
            display_name: "Resize Window".to_string(),
            category: "Actions".to_string(),
            description: "Resize a window to specified dimensions".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to resize".to_string(),
                },
                PinDefinition {
                    name: "Width".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("800".to_string()),
                    description: "New width".to_string(),
                },
                PinDefinition {
                    name: "Height".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("600".to_string()),
                    description: "New height".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::ResizeWindow as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "SetWindowTitle".to_string(),
            display_name: "Set Window Title".to_string(),
            category: "Actions".to_string(),
            description: "Change a window's title".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to modify".to_string(),
                },
                PinDefinition {
                    name: "Title".to_string(),
                    r#type: PinType::String as i32,
                    required: true,
                    default_value: Some("New Title".to_string()),
                    description: "New window title".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::SetWindowTitle as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "AnimateWindow".to_string(),
            display_name: "Animate Window".to_string(),
            category: "Actions".to_string(),
            description: "Animate a window with effects".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to animate".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "animation_type".to_string(),
                r#type: PropertyType::EnumProperty as i32,
                display_name: "Animation Type".to_string(),
                default_value: Some("Fade".to_string()),
                description: "Type of animation".to_string(),
                enum_values: vec![
                    "Fade".to_string(),
                    "Slide".to_string(),
                    "Scale".to_string(),
                    "Expand".to_string(),
                    "Contract".to_string(),
                ],
            }],
            legacy_type: NodeType::AnimateWindow as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "BringToFront".to_string(),
            display_name: "Bring To Front".to_string(),
            category: "Actions".to_string(),
            description: "Bring a window to the front (raise it)".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to bring forward".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::BringToFront as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "SendToBack".to_string(),
            display_name: "Send To Back".to_string(),
            category: "Actions".to_string(),
            description: "Send a window to the back (lower it)".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to send back".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::SendToBack as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "SetAlwaysOnTop".to_string(),
            display_name: "Set Always On Top".to_string(),
            category: "Actions".to_string(),
            description: "Make a window stay on top of others".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to modify".to_string(),
                },
                PinDefinition {
                    name: "Enabled".to_string(),
                    r#type: PinType::Bool as i32,
                    required: true,
                    default_value: Some("true".to_string()),
                    description: "Enable or disable always on top".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::SetAlwaysOnTop as i32,
        });

        // ====================================================================
        // Data Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "GetAllWindows".to_string(),
            display_name: "Get All Windows".to_string(),
            category: "Data".to_string(),
            description: "Get list of all managed windows".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this query".to_string(),
            }],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Windows".to_string(),
                    r#type: PinType::WindowList as i32,
                    required: false,
                    default_value: None,
                    description: "List of windows".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetAllWindows as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetWindowPosition".to_string(),
            display_name: "Get Window Position".to_string(),
            category: "Data".to_string(),
            description: "Get the position of a window".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this query".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to query".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "X".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "X coordinate".to_string(),
                },
                PinDefinition {
                    name: "Y".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Y coordinate".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetWindowPosition as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetWindowSize".to_string(),
            display_name: "Get Window Size".to_string(),
            category: "Data".to_string(),
            description: "Get the size of a window".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this query".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to query".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Width".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Window width".to_string(),
                },
                PinDefinition {
                    name: "Height".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Window height".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetWindowSize as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetActiveWindow".to_string(),
            display_name: "Get Active Window".to_string(),
            category: "Data".to_string(),
            description: "Get the currently focused window".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this query".to_string(),
            }],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: false,
                    default_value: None,
                    description: "Active window".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetActiveWindow as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetWindowInfo".to_string(),
            display_name: "Get Window Info".to_string(),
            category: "Data".to_string(),
            description: "Get detailed information about a window".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this query".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to query".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Title".to_string(),
                    r#type: PinType::String as i32,
                    required: false,
                    default_value: None,
                    description: "Window title".to_string(),
                },
                PinDefinition {
                    name: "Class".to_string(),
                    r#type: PinType::String as i32,
                    required: false,
                    default_value: None,
                    description: "Window class".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetWindowInfo as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetMonitorForWindow".to_string(),
            display_name: "Get Monitor For Window".to_string(),
            category: "Data".to_string(),
            description: "Get which monitor a window is on".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this query".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to query".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Monitor".to_string(),
                    r#type: PinType::Monitor as i32,
                    required: false,
                    default_value: None,
                    description: "Monitor handle".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetMonitorForWindow as i32,
        });

        // ====================================================================
        // Condition Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "WindowExists".to_string(),
            display_name: "Window Exists".to_string(),
            category: "Conditions".to_string(),
            description: "Check if a window exists".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this check".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to check".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "True".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window exists".to_string(),
                },
                PinDefinition {
                    name: "False".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window doesn't exist".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::WindowExists as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "WindowHasTitle".to_string(),
            display_name: "Window Has Title".to_string(),
            category: "Conditions".to_string(),
            description: "Check if a window's title matches a pattern".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this check".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to check".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "True".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Title matches".to_string(),
                },
                PinDefinition {
                    name: "False".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Title doesn't match".to_string(),
                },
            ],
            properties: vec![PropertyDefinition {
                key: "title".to_string(),
                r#type: PropertyType::StringProperty as i32,
                display_name: "Title Pattern".to_string(),
                default_value: Some("".to_string()),
                description: "Title to match".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::WindowHasTitle as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "MatchWindow".to_string(),
            display_name: "Match Window".to_string(),
            category: "Conditions".to_string(),
            description: "Match a window using flexible criteria".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this check".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to check".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "True".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window matches".to_string(),
                },
                PinDefinition {
                    name: "False".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window doesn't match".to_string(),
                },
            ],
            properties: vec![
                PropertyDefinition {
                    key: "match_kind".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Kind".to_string(),
                    default_value: Some("Title".to_string()),
                    description: "What to match".to_string(),
                    enum_values: vec![
                        "Title".to_string(),
                        "Class".to_string(),
                        "Exe".to_string(),
                        "Path".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "match_strategy".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Strategy".to_string(),
                    default_value: Some("Contains".to_string()),
                    description: "How to match".to_string(),
                    enum_values: vec![
                        "Equals".to_string(),
                        "Contains".to_string(),
                        "StartsWith".to_string(),
                        "EndsWith".to_string(),
                        "Regex".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "pattern".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Pattern".to_string(),
                    default_value: Some("".to_string()),
                    description: "Pattern to match".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::MatchWindow as i32,
        });

        // ====================================================================
        // Math Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "Add".to_string(),
            display_name: "Add".to_string(),
            category: "Math".to_string(),
            description: "Add two numbers".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "A".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "First number".to_string(),
                },
                PinDefinition {
                    name: "B".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "Second number".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Result".to_string(),
                r#type: PinType::Number as i32,
                required: false,
                default_value: None,
                description: "Sum".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::Add as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "Subtract".to_string(),
            display_name: "Subtract".to_string(),
            category: "Math".to_string(),
            description: "Subtract two numbers".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "A".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "First number".to_string(),
                },
                PinDefinition {
                    name: "B".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("0".to_string()),
                    description: "Second number".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Result".to_string(),
                r#type: PinType::Number as i32,
                required: false,
                default_value: None,
                description: "Difference".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::Subtract as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "Multiply".to_string(),
            display_name: "Multiply".to_string(),
            category: "Math".to_string(),
            description: "Multiply two numbers".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "A".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("1".to_string()),
                    description: "First number".to_string(),
                },
                PinDefinition {
                    name: "B".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("1".to_string()),
                    description: "Second number".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Result".to_string(),
                r#type: PinType::Number as i32,
                required: false,
                default_value: None,
                description: "Product".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::Multiply as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "Divide".to_string(),
            display_name: "Divide".to_string(),
            category: "Math".to_string(),
            description: "Divide two numbers".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "A".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("1".to_string()),
                    description: "Dividend".to_string(),
                },
                PinDefinition {
                    name: "B".to_string(),
                    r#type: PinType::Number as i32,
                    required: true,
                    default_value: Some("1".to_string()),
                    description: "Divisor".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Result".to_string(),
                r#type: PinType::Number as i32,
                required: false,
                default_value: None,
                description: "Quotient".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::Divide as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetMonitors".to_string(),
            display_name: "Get Monitors".to_string(),
            category: "Monitors".to_string(),
            description: "Get list of all monitors".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this query".to_string(),
            }],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Monitors".to_string(),
                    r#type: PinType::MonitorList as i32,
                    required: false,
                    default_value: None,
                    description: "List of monitors".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetMonitors as i32,
        });

        // ====================================================================
        // Helper Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "ForEach".to_string(),
            display_name: "For Each".to_string(),
            category: "Control Flow".to_string(),
            description: "Iterate over a list and execute for each item".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Start loop".to_string(),
                },
                PinDefinition {
                    name: "List".to_string(),
                    r#type: PinType::WindowList as i32, // Can be any list type
                    required: true,
                    default_value: None,
                    description: "List to iterate".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Loop Body".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Execute for each item".to_string(),
                },
                PinDefinition {
                    name: "Item".to_string(),
                    r#type: PinType::WindowHandle as i32, // Current item
                    required: false,
                    default_value: None,
                    description: "Current item".to_string(),
                },
                PinDefinition {
                    name: "Completed".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "After loop completes".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::ForEach as i32,
        });

        // ====================================================================
        // Workspace Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "CreateWorkspace".to_string(),
            display_name: "Create Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Create a new workspace on a monitor".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Monitor".to_string(),
                    r#type: PinType::Monitor as i32,
                    required: true,
                    default_value: None,
                    description: "Monitor to create workspace on".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "workspace_name".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Workspace Name".to_string(),
                    default_value: Some("Workspace".to_string()),
                    description: "Name for the workspace".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "layout".to_string(),
                    r#type: PropertyType::LayoutProperty as i32,
                    display_name: "Layout".to_string(),
                    default_value: Some("BSP".to_string()),
                    description: "Default layout type".to_string(),
                    enum_values: vec![
                        "BSP".to_string(),
                        "Grid".to_string(),
                        "Columns".to_string(),
                        "Rows".to_string(),
                        "Monocle".to_string(),
                    ],
                },
            ],
            legacy_type: NodeType::CreateWorkspace as i32,
        });

        // ====================================================================
        // Value Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "NumberValue".to_string(),
            display_name: "Number".to_string(),
            category: "Values".to_string(),
            description: "A constant number value".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Value".to_string(),
                r#type: PinType::Number as i32,
                required: false,
                default_value: Some("0".to_string()),
                description: "Number value".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "value".to_string(),
                r#type: PropertyType::NumberProperty as i32,
                display_name: "Value".to_string(),
                default_value: Some("0".to_string()),
                description: "The number value".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::NumberValue as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "StringValue".to_string(),
            display_name: "String".to_string(),
            category: "Values".to_string(),
            description: "A constant string value".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Value".to_string(),
                r#type: PinType::String as i32,
                required: false,
                default_value: Some("".to_string()),
                description: "String value".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "value".to_string(),
                r#type: PropertyType::StringProperty as i32,
                display_name: "Value".to_string(),
                default_value: Some("".to_string()),
                description: "The string value".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::StringValue as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "BoolValue".to_string(),
            display_name: "Boolean".to_string(),
            category: "Values".to_string(),
            description: "A constant boolean value".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Value".to_string(),
                r#type: PinType::Bool as i32,
                required: false,
                default_value: Some("false".to_string()),
                description: "Boolean value".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "value".to_string(),
                r#type: PropertyType::BoolProperty as i32,
                display_name: "Value".to_string(),
                default_value: Some("false".to_string()),
                description: "The boolean value".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::BoolValue as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "ColorValue".to_string(),
            display_name: "Color".to_string(),
            category: "Values".to_string(),
            description: "A constant color value (RGB)".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Value".to_string(),
                r#type: PinType::Color as i32,
                required: false,
                default_value: None,
                description: "Color value".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "r".to_string(),
                    r#type: PropertyType::NumberProperty as i32,
                    display_name: "Red".to_string(),
                    default_value: Some("255".to_string()),
                    description: "Red component (0-255)".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "g".to_string(),
                    r#type: PropertyType::NumberProperty as i32,
                    display_name: "Green".to_string(),
                    default_value: Some("0".to_string()),
                    description: "Green component (0-255)".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "b".to_string(),
                    r#type: PropertyType::NumberProperty as i32,
                    display_name: "Blue".to_string(),
                    default_value: Some("0".to_string()),
                    description: "Blue component (0-255)".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::ColorValue as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "BorderStyleValue".to_string(),
            display_name: "Border Style".to_string(),
            category: "Values".to_string(),
            description: "A constant border style value".to_string(),
            inputs: vec![],
            outputs: vec![PinDefinition {
                name: "Value".to_string(),
                r#type: PinType::BorderStyle as i32,
                required: false,
                default_value: None,
                description: "Border style value".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "value".to_string(),
                r#type: PropertyType::EnumProperty as i32,
                display_name: "Style".to_string(),
                default_value: Some("Square".to_string()),
                description: "Border corner style".to_string(),
                enum_values: vec!["Square".to_string(), "Rounded".to_string()],
            }],
            legacy_type: NodeType::BorderStyleValue as i32,
        });

        // ====================================================================
        // Helper Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "WindowListToSingle".to_string(),
            display_name: "Window List To Single".to_string(),
            category: "Helpers".to_string(),
            description: "Extract a single window from a list by index".to_string(),
            inputs: vec![PinDefinition {
                name: "List".to_string(),
                r#type: PinType::WindowList as i32,
                required: true,
                default_value: None,
                description: "Window list".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Window".to_string(),
                r#type: PinType::WindowHandle as i32,
                required: false,
                default_value: None,
                description: "Selected window".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "index".to_string(),
                r#type: PropertyType::NumberProperty as i32,
                display_name: "Index".to_string(),
                default_value: Some("0".to_string()),
                description: "Index in list (0-based)".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::WindowListToSingle as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "CreateRule".to_string(),
            display_name: "Create Rule".to_string(),
            category: "Helpers".to_string(),
            description: "Create a single rule from pattern and match criteria".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Pattern".to_string(),
                    r#type: PinType::String as i32,
                    required: false,
                    default_value: Some("".to_string()),
                    description: "Pattern to match".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Rule".to_string(),
                    r#type: PinType::Rule as i32,
                    required: false,
                    default_value: None,
                    description: "The created rule".to_string(),
                },
            ],
            properties: vec![
                PropertyDefinition {
                    key: "pattern".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Pattern".to_string(),
                    default_value: Some("".to_string()),
                    description: "Pattern to match (used if pin not connected)".to_string(),
                    enum_values: vec![],
                },
                PropertyDefinition {
                    key: "match_kind".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Kind".to_string(),
                    default_value: Some("Title".to_string()),
                    description: "What to match".to_string(),
                    enum_values: vec![
                        "Title".to_string(),
                        "Class".to_string(),
                        "Exe".to_string(),
                        "Path".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "match_strategy".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Strategy".to_string(),
                    default_value: Some("Contains".to_string()),
                    description: "How to match".to_string(),
                    enum_values: vec![
                        "Equals".to_string(),
                        "Contains".to_string(),
                        "StartsWith".to_string(),
                        "EndsWith".to_string(),
                        "Regex".to_string(),
                    ],
                },
            ],
            legacy_type: NodeType::CreateRule as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "CombineRules".to_string(),
            display_name: "Combine Rules".to_string(),
            category: "Helpers".to_string(),
            description: "Combine multiple rules into a rule list".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Rule 1".to_string(),
                    r#type: PinType::Rule as i32,
                    required: false,
                    default_value: None,
                    description: "First rule".to_string(),
                },
                PinDefinition {
                    name: "Rule 2".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Second rule".to_string(),
                },
                PinDefinition {
                    name: "Rule 3".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Third rule".to_string(),
                },
                PinDefinition {
                    name: "Rule 4".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Fourth rule".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Rules".to_string(),
                    r#type: PinType::RuleList as i32,
                    required: false,
                    default_value: None,
                    description: "Combined rule list".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::CombineRules as i32,
        });

        // ====================================================================
        // Workspace Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "SwitchToWorkspace".to_string(),
            display_name: "Switch To Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Switch to a different workspace".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "workspace_index".to_string(),
                r#type: PropertyType::NumberProperty as i32,
                display_name: "Workspace Index".to_string(),
                default_value: Some("0".to_string()),
                description: "Workspace number to switch to".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::SwitchToWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "MoveWindowToWorkspace".to_string(),
            display_name: "Move Window To Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Move a window to a different workspace".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to move".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "workspace_index".to_string(),
                r#type: PropertyType::NumberProperty as i32,
                display_name: "Workspace Index".to_string(),
                default_value: Some("0".to_string()),
                description: "Target workspace number".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::MoveWindowToWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetCurrentWorkspace".to_string(),
            display_name: "Get Current Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Get the current workspace index".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this query".to_string(),
            }],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Index".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Current workspace index".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetCurrentWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "NextWorkspace".to_string(),
            display_name: "Next Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Switch to the next workspace".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::NextWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "PreviousWorkspace".to_string(),
            display_name: "Previous Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Switch to the previous workspace".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::PreviousWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "FocusWorkspace".to_string(),
            display_name: "Focus Workspace".to_string(),
            category: "Workspaces".to_string(),
            description: "Focus a specific workspace by index".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![PropertyDefinition {
                key: "workspace_index".to_string(),
                r#type: PropertyType::NumberProperty as i32,
                display_name: "Workspace Index".to_string(),
                default_value: Some("0".to_string()),
                description: "Workspace to focus".to_string(),
                enum_values: vec![],
            }],
            legacy_type: NodeType::FocusWorkspace as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "GetMonitorInfo".to_string(),
            display_name: "Get Monitor Info".to_string(),
            category: "Monitors".to_string(),
            description: "Get detailed information about a monitor".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this query".to_string(),
                },
                PinDefinition {
                    name: "Monitor".to_string(),
                    r#type: PinType::Monitor as i32,
                    required: true,
                    default_value: None,
                    description: "Monitor to query".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Then".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Continue execution".to_string(),
                },
                PinDefinition {
                    name: "Name".to_string(),
                    r#type: PinType::String as i32,
                    required: false,
                    default_value: None,
                    description: "Monitor name".to_string(),
                },
                PinDefinition {
                    name: "Width".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Monitor width".to_string(),
                },
                PinDefinition {
                    name: "Height".to_string(),
                    r#type: PinType::Number as i32,
                    required: false,
                    default_value: None,
                    description: "Monitor height".to_string(),
                },
            ],
            properties: vec![],
            legacy_type: NodeType::GetMonitorInfo as i32,
        });

        // ====================================================================
        // Rule Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "CreateFloatRule".to_string(),
            display_name: "Create Float Rule".to_string(),
            category: "Rules".to_string(),
            description: "Create a rule to float windows matching criteria".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "match_kind".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Kind".to_string(),
                    default_value: Some("Title".to_string()),
                    description: "What to match".to_string(),
                    enum_values: vec![
                        "Title".to_string(),
                        "Class".to_string(),
                        "Exe".to_string(),
                        "Path".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "match_strategy".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Strategy".to_string(),
                    default_value: Some("Contains".to_string()),
                    description: "How to match".to_string(),
                    enum_values: vec![
                        "Equals".to_string(),
                        "Contains".to_string(),
                        "StartsWith".to_string(),
                        "EndsWith".to_string(),
                        "Regex".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "pattern".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Pattern".to_string(),
                    default_value: Some("".to_string()),
                    description: "Pattern to match".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::CreateFloatRule as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "CreateIgnoreRule".to_string(),
            display_name: "Create Ignore Rule".to_string(),
            category: "Rules".to_string(),
            description: "Create a rule to ignore windows matching criteria".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "match_kind".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Kind".to_string(),
                    default_value: Some("Title".to_string()),
                    description: "What to match".to_string(),
                    enum_values: vec![
                        "Title".to_string(),
                        "Class".to_string(),
                        "Exe".to_string(),
                        "Path".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "match_strategy".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Strategy".to_string(),
                    default_value: Some("Contains".to_string()),
                    description: "How to match".to_string(),
                    enum_values: vec![
                        "Equals".to_string(),
                        "Contains".to_string(),
                        "StartsWith".to_string(),
                        "EndsWith".to_string(),
                        "Regex".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "pattern".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Pattern".to_string(),
                    default_value: Some("".to_string()),
                    description: "Pattern to match".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::CreateIgnoreRule as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "CreateManageRule".to_string(),
            display_name: "Create Manage Rule".to_string(),
            category: "Rules".to_string(),
            description: "Create a rule to manage windows matching criteria".to_string(),
            inputs: vec![PinDefinition {
                name: "Trigger".to_string(),
                r#type: PinType::Execution as i32,
                required: true,
                default_value: None,
                description: "Execute this action".to_string(),
            }],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![
                PropertyDefinition {
                    key: "match_kind".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Kind".to_string(),
                    default_value: Some("Title".to_string()),
                    description: "What to match".to_string(),
                    enum_values: vec![
                        "Title".to_string(),
                        "Class".to_string(),
                        "Exe".to_string(),
                        "Path".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "match_strategy".to_string(),
                    r#type: PropertyType::EnumProperty as i32,
                    display_name: "Match Strategy".to_string(),
                    default_value: Some("Contains".to_string()),
                    description: "How to match".to_string(),
                    enum_values: vec![
                        "Equals".to_string(),
                        "Contains".to_string(),
                        "StartsWith".to_string(),
                        "EndsWith".to_string(),
                        "Regex".to_string(),
                    ],
                },
                PropertyDefinition {
                    key: "pattern".to_string(),
                    r#type: PropertyType::StringProperty as i32,
                    display_name: "Pattern".to_string(),
                    default_value: Some("".to_string()),
                    description: "Pattern to match".to_string(),
                    enum_values: vec![],
                },
            ],
            legacy_type: NodeType::CreateManageRule as i32,
        });

        definitions.push(NodeDefinition {
            type_id: "WindowModeSelector".to_string(),
            display_name: "Window Mode Selector".to_string(),
            category: "Rules".to_string(),
            description: "Select window mode based on built-in rules".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Window".to_string(),
                    r#type: PinType::WindowHandle as i32,
                    required: true,
                    default_value: None,
                    description: "Window to evaluate".to_string(),
                },
            ],
            outputs: vec![
                PinDefinition {
                    name: "Tiled".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window should be tiled".to_string(),
                },
                PinDefinition {
                    name: "Floating".to_string(),
                    r#type: PinType::Execution as i32,
                    required: false,
                    default_value: None,
                    description: "Window should float".to_string(),
                },
            ],
            properties: vec![PropertyDefinition {
                key: "default_mode".to_string(),
                r#type: PropertyType::EnumProperty as i32,
                display_name: "Default Mode".to_string(),
                default_value: Some("Tiled".to_string()),
                description: "Mode when no rule matches".to_string(),
                enum_values: vec!["Tiled".to_string(), "Floating".to_string()],
            }],
            legacy_type: NodeType::WindowModeSelector as i32,
        });

        // ====================================================================
        // Debug Nodes
        // ====================================================================

        definitions.push(NodeDefinition {
            type_id: "LogMessage".to_string(),
            display_name: "Log Message".to_string(),
            category: "Debug".to_string(),
            description: "Print a message to the log".to_string(),
            inputs: vec![
                PinDefinition {
                    name: "Trigger".to_string(),
                    r#type: PinType::Execution as i32,
                    required: true,
                    default_value: None,
                    description: "Execute this action".to_string(),
                },
                PinDefinition {
                    name: "Message".to_string(),
                    r#type: PinType::String as i32,
                    required: true,
                    default_value: Some("Hello".to_string()),
                    description: "Message to log".to_string(),
                },
            ],
            outputs: vec![PinDefinition {
                name: "Then".to_string(),
                r#type: PinType::Execution as i32,
                required: false,
                default_value: None,
                description: "Continue execution".to_string(),
            }],
            properties: vec![],
            legacy_type: NodeType::LogMessage as i32,
        });

        definitions
    }

    /// Get all node definitions
    pub fn get_definitions(&self) -> &[NodeDefinition] {
        &self.definitions
    }

    /// Get a specific node definition by type_id
    pub fn get_definition(&self, type_id: &str) -> Option<&NodeDefinition> {
        self.definitions.iter().find(|d| d.type_id == type_id)
    }

    /// Get node definitions grouped by category
    pub fn get_by_category(&self) -> std::collections::HashMap<String, Vec<&NodeDefinition>> {
        let mut map = std::collections::HashMap::new();
        for def in &self.definitions {
            map.entry(def.category.clone())
                .or_insert_with(Vec::new)
                .push(def);
        }
        map
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use gevrek_model::graph::{node_catalog, NodeGraph};

    /// The registry describes exactly the nodes the model has, with the
    /// model's pins, names and groups.
    #[test]
    fn the_registry_describes_every_catalog_node_with_the_models_pins() {
        let registry = NodeRegistry::new();
        let mut expected = 0;
        for (group, node_types) in node_catalog() {
            for node_type in node_types {
                expected += 1;
                let type_id = type_id_of(&node_type);
                let definition = registry
                    .get_definition(&type_id)
                    .unwrap_or_else(|| panic!("no definition for {type_id}"));
                let mut graph = NodeGraph::new();
                let id = graph.add_node(node_type.clone(), [0.0, 0.0]);
                let node = &graph.nodes[&id];
                let names = |pins: &[gevrek_model::graph::Pin]| {
                    pins.iter().map(|pin| pin.name.clone()).collect::<Vec<_>>()
                };
                let defined = |pins: &[PinDefinition]| {
                    pins.iter().map(|pin| pin.name.clone()).collect::<Vec<_>>()
                };
                assert_eq!(
                    defined(&definition.inputs),
                    names(&node.inputs),
                    "{type_id}"
                );
                assert_eq!(
                    defined(&definition.outputs),
                    names(&node.outputs),
                    "{type_id}"
                );
                assert_eq!(definition.display_name, node_type.display_name());
                assert_eq!(definition.category, group);
                assert!(definition
                    .inputs
                    .iter()
                    .chain(&definition.outputs)
                    .all(|pin| pin.r#type != PinType::Unspecified as i32));
            }
        }
        assert_eq!(registry.get_definitions().len(), expected);
    }

    /// The editor shows the description when hovering a node in the menu.
    #[test]
    fn every_node_has_a_description() {
        let registry = NodeRegistry::new();
        let undescribed: Vec<_> = registry
            .get_definitions()
            .iter()
            .filter(|definition| definition.description.trim().is_empty())
            .map(|definition| definition.type_id.clone())
            .collect();
        assert!(undescribed.is_empty(), "no description for {undescribed:?}");
    }

    /// Every hand-written help text belongs to a node that exists.
    #[test]
    fn no_hand_written_description_is_orphaned() {
        let registry = NodeRegistry::new();
        for described in NodeRegistry::hand_written() {
            assert!(
                registry.get_definition(&described.type_id).is_some(),
                "{} is described but not in the catalog",
                described.type_id
            );
        }
    }
}
