//! Explicit live smoke test. The tool result is synthetic; generated code is never executed.
use crate::session::Session;
use serde_json::{Value, json};

pub async fn run(session: &Session) -> Result<(), Box<dyn std::error::Error>> {
    let token = &session.token;
    let url = format!("{}/responses", session.url);
    let tool = json!({"type":"custom", "name":"exec", "description":"Return the supplied JavaScript unchanged to a synthetic test runner.",
               "format":{"type":"grammar","syntax":"lark","definition":"start: /[\\s\\S]+/"}});
    let mut request = json!({"model":"gpt-6-astra", "input":[{"role":"user", "content":"Call exec with exactly text(\"BRIDGE_TOOL_OK\"); . Do not invoke nested tools. After its output, reply with that output only."}], "tools":[tool], "max_output_tokens":2048});
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&request)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err("Adapter rejected smoke request".into());
    }
    let first: Value = response.json().await?;
    if first["status"] != "completed" {
        return Err("First inference did not complete".into());
    }
    let output = first["output"].as_array().ok_or("Missing output")?;
    let calls: Vec<_> = output
        .iter()
        .filter(|item| item["type"] == "custom_tool_call")
        .collect();
    if calls.len() != 1 || calls[0]["name"] != "exec" {
        return Err("Expected one exec call".into());
    }
    let raw = calls[0]["input"]
        .as_str()
        .ok_or("Missing custom input")?
        .trim();
    if !matches!(
        raw,
        "text(\"BRIDGE_TOOL_OK\");" | "text(\"BRIDGE_TOOL_OK\")"
    ) {
        return Err("Unexpected test call input; nothing executed".into());
    }
    let call_id = calls[0]["call_id"].clone();
    let history = request["input"].as_array_mut().unwrap();
    history.extend(output.iter().cloned());
    history.push(
        json!({"type":"custom_tool_call_output","call_id":call_id,"output":"BRIDGE_TOOL_OK"}),
    );
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&request)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err("Adapter rejected replay request".into());
    }
    let final_response: Value = response.json().await?;
    if final_response["status"] != "completed" {
        return Err("Replay did not complete".into());
    }
    if final_response["output"]
        .as_array()
        .ok_or("Missing replay output")?
        .iter()
        .any(|item| !matches!(item["type"].as_str(), Some("message" | "reasoning")))
    {
        return Err("Replay requested another tool call".into());
    }
    let text: String = final_response["output"]
        .as_array()
        .ok_or("Missing replay output")?
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect();
    if text.trim() != "BRIDGE_TOOL_OK" {
        return Err("Unexpected replay answer".into());
    }
    println!("Custom-tool call and result continuation passed. No generated code executed.");
    Ok(())
}
