#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Function,
    Custom,
}

impl ToolKind {
    pub(crate) fn from_definition(kind: &str) -> Option<Self> {
        match kind {
            "function" => Some(Self::Function),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub(crate) fn from_call(kind: &str) -> Option<Self> {
        match kind {
            "function_call" => Some(Self::Function),
            "custom_tool_call" => Some(Self::Custom),
            _ => None,
        }
    }

    pub(crate) fn from_output(kind: &str) -> Option<Self> {
        match kind {
            "function_call_output" => Some(Self::Function),
            "custom_tool_call_output" => Some(Self::Custom),
            _ => None,
        }
    }

    pub(crate) fn input_key(self) -> &'static str {
        match self {
            Self::Function => "arguments",
            Self::Custom => "input",
        }
    }

    pub(crate) fn call_type(self) -> &'static str {
        match self {
            Self::Function => "function_call",
            Self::Custom => "custom_tool_call",
        }
    }

    pub(crate) fn id_prefix(self) -> &'static str {
        match self {
            Self::Function => "fc",
            Self::Custom => "ctc",
        }
    }

    pub(crate) fn input_event(self, suffix: &str) -> String {
        let prefix = match self {
            Self::Function => "response.function_call_arguments",
            Self::Custom => "response.custom_tool_call_input",
        };
        format!("{prefix}.{suffix}")
    }

    pub(crate) fn call_fields(self) -> &'static [&'static str] {
        match self {
            Self::Function => &["type", "id", "call_id", "name", "arguments", "status"],
            Self::Custom => &["type", "id", "call_id", "name", "input", "status"],
        }
    }
}
