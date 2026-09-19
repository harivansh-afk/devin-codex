//! Measured subset of Cognition's GetChatMessage protobuf schema.
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct Metadata {
    #[prost(string, tag = "1")]
    pub client_name: String,
    #[prost(string, tag = "2")]
    pub client_version: String,
    #[prost(string, tag = "3")]
    pub api_key: String,
    #[prost(string, tag = "4")]
    pub locale: String,
    #[prost(string, tag = "5")]
    pub platform: String,
    #[prost(string, tag = "7")]
    pub ide_version: String,
    #[prost(string, tag = "12")]
    pub extension_name: String,
    #[prost(string, tag = "28")]
    pub ide_type: String,
    #[prost(string, tag = "31")]
    pub device_id: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct CompletionConfig {
    #[prost(uint64, tag = "1")]
    pub num_completions: u64,
    #[prost(uint64, tag = "2")]
    pub max_tokens: u64,
    #[prost(uint64, tag = "3")]
    pub max_newlines: u64,
    #[prost(double, tag = "5")]
    pub temperature: f64,
    #[prost(uint64, tag = "7")]
    pub top_k: u64,
    #[prost(double, tag = "8")]
    pub top_p: f64,
}
#[derive(Clone, PartialEq, Message)]
pub struct ToolChoice {
    #[prost(string, tag = "1")]
    pub mode: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Trajectory {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(int32, tag = "3")]
    pub source: i32,
    #[prost(int32, tag = "4")]
    pub node_index: i32,
}
#[derive(Clone, PartialEq, Message)]
pub struct ToolDefinition {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub description: String,
    #[prost(string, tag = "3")]
    pub parameters_json: String,
    #[prost(bool, tag = "9")]
    pub is_custom: bool,
    #[prost(string, tag = "10")]
    pub grammar: String,
    #[prost(string, tag = "11")]
    pub grammar_syntax: String,
    #[prost(bool, optional, tag = "12")]
    pub strict: Option<bool>,
}
#[derive(Clone, PartialEq, Message)]
pub struct ToolCall {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub arguments_json: String,
    #[prost(string, tag = "4")]
    pub invalid_json_str: String,
    #[prost(string, tag = "5")]
    pub invalid_json_err: String,
    #[prost(bool, optional, tag = "6")]
    pub is_custom_tool_call: Option<bool>,
}
#[derive(Clone, PartialEq, Message)]
pub struct ChatMessage {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(int32, tag = "2")]
    pub role: i32,
    #[prost(string, tag = "3")]
    pub content: String,
    #[prost(message, repeated, tag = "6")]
    pub tool_calls: Vec<ToolCall>,
    #[prost(string, tag = "7")]
    pub tool_call_id: String,
    #[prost(string, tag = "12")]
    pub reasoning: String,
    #[prost(string, tag = "18")]
    pub reasoning_type: String,
    #[prost(string, tag = "19")]
    pub phase: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct ChatRequest {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<Metadata>,
    #[prost(string, tag = "2")]
    pub system_prompt: String,
    #[prost(message, repeated, tag = "3")]
    pub messages: Vec<ChatMessage>,
    #[prost(int32, tag = "7")]
    pub request_type: i32,
    #[prost(message, optional, tag = "8")]
    pub completion_config: Option<CompletionConfig>,
    #[prost(message, repeated, tag = "10")]
    pub tools: Vec<ToolDefinition>,
    #[prost(bool, tag = "11")]
    pub disable_parallel_tool_calls: bool,
    #[prost(message, optional, tag = "12")]
    pub tool_choice: Option<ToolChoice>,
    #[prost(message, optional, tag = "15")]
    pub trajectory: Option<Trajectory>,
    #[prost(string, tag = "16")]
    pub cascade_id: String,
    #[prost(int32, tag = "20")]
    pub planner_mode: i32,
    #[prost(string, tag = "21")]
    pub model: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Usage {
    #[prost(uint64, optional, tag = "2")]
    pub input_tokens: Option<u64>,
    #[prost(uint64, optional, tag = "3")]
    pub output_tokens: Option<u64>,
    #[prost(uint64, optional, tag = "4")]
    pub cache_write_tokens: Option<u64>,
    #[prost(uint64, optional, tag = "5")]
    pub cache_read_tokens: Option<u64>,
}
#[derive(Clone, PartialEq, Message)]
pub struct ChatResponse {
    #[prost(string, tag = "1")]
    pub message_id: String,
    #[prost(string, tag = "3")]
    pub delta_text: String,
    #[prost(int32, optional, tag = "5")]
    pub stop_reason: Option<i32>,
    #[prost(message, repeated, tag = "6")]
    pub delta_tool_calls: Vec<ToolCall>,
    #[prost(message, optional, tag = "7")]
    pub usage: Option<Usage>,
    #[prost(bool, tag = "8")]
    pub redact: bool,
    #[prost(string, tag = "9")]
    pub delta_thinking: String,
    #[prost(string, tag = "10")]
    pub delta_signature: String,
    #[prost(bool, tag = "11")]
    pub thinking_redacted: bool,
    #[prost(string, tag = "15")]
    pub output_id: String,
    #[prost(string, tag = "16")]
    pub thinking_id: String,
    #[prost(string, tag = "21")]
    pub delta_signature_type: String,
    #[prost(double, optional, tag = "22")]
    pub committed_acu_cost: Option<f64>,
    #[prost(string, tag = "23")]
    pub actual_model_uid: String,
    #[prost(string, tag = "25")]
    pub phase: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_response_tags_and_presence() {
        let response = ChatResponse::decode(&b"\x1a\x02hi\x28\x00\x32\x02\x30\x00"[..]).unwrap();
        assert_eq!(response.delta_text, "hi");
        assert_eq!(response.stop_reason, Some(0));
        assert_eq!(
            response.delta_tool_calls[0].is_custom_tool_call,
            Some(false)
        );
        assert_eq!(ChatResponse::decode(&b""[..]).unwrap().stop_reason, None);
    }
    #[test]
    fn malformed_protobuf_rejected() {
        for input in [&b"\x1a\x08hi"[..], &b"\x1a\x01\xff"[..], &b"\x80"[..]] {
            assert!(ChatResponse::decode(input).is_err());
        }
    }
}
