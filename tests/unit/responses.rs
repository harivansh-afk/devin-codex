use super::*;
use crate::proto::Usage;

fn translator() -> Translator {
    Translator::new(
        "gpt-6-astra".into(),
        HashMap::from([
            ("exec".into(), ToolKind::Custom),
            ("run".into(), ToolKind::Function),
        ]),
        true,
        Mode::Strict,
    )
}

fn call(custom: bool) -> ChatResponse {
    ChatResponse {
        delta_tool_calls: vec![ToolCall {
            id: "a".into(),
            name: if custom { "exec" } else { "run" }.into(),
            is_custom_tool_call: Some(custom),
            arguments_json: if custom { "text(" } else { "{" }.into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn reasoning() -> Value {
    json!({"type":"reasoning","id":"rs_native","summary":[],"content":[],"status":"","encrypted_content":"opaque"})
}

#[test]
fn text_phase_sequences_and_success_without_stop_enum() {
    let mut t = translator();
    let mut events = t.created_events();
    t.push(ChatResponse {
        delta_text: "hello".into(),
        phase: "final_answer".into(),
        ..Default::default()
    })
    .unwrap();
    events.extend(t.finish().unwrap());
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], index);
    }
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["phase"],
        "final_answer"
    );
}

#[test]
fn custom_identity_only_on_first_delta() {
    let mut t = translator();
    t.push(call(true)).unwrap();
    t.push(ChatResponse {
        delta_tool_calls: vec![ToolCall {
            arguments_json: "42)".into(),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    let item = &events.last().unwrap()["response"]["output"][0];
    assert_eq!(item["input"], "text(42)");
    assert!(item.get("arguments").is_none());
}

#[test]
fn tool_event_lifecycle_preserves_payloads_and_withholds_incomplete_calls() {
    for (custom, name, item_type, key, delta_event, done_event, payload) in [
        (
            false,
            "run",
            "function_call",
            "arguments",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "{\"command\":\"echo hello\"}",
        ),
        (
            true,
            "exec",
            "custom_tool_call",
            "input",
            "response.custom_tool_call_input.delta",
            "response.custom_tool_call_input.done",
            "text('hello')",
        ),
    ] {
        for stop in [None, Some(3), Some(11)] {
            let mut t = translator();
            let mut events = t.created_events();
            t.push(ChatResponse {
                delta_tool_calls: vec![ToolCall {
                    id: "call-original".into(),
                    name: name.into(),
                    arguments_json: payload.into(),
                    is_custom_tool_call: Some(custom),
                    ..Default::default()
                }],
                stop_reason: stop,
                ..Default::default()
            })
            .unwrap();
            events.extend(t.finish().unwrap());
            for (index, event) in events.iter().enumerate() {
                assert_eq!(event["sequence_number"], index);
            }
            let response = &events.last().unwrap()["response"];
            let item = &response["output"][0];
            assert_eq!(item["type"], item_type);
            assert_eq!(item["call_id"], "call-original");
            assert_eq!(item["name"], name);
            assert_eq!(item[key], payload);
            if let Some(stop) = stop {
                assert_eq!(events.len(), 3);
                assert_eq!(events[2]["type"], "response.incomplete");
                assert_eq!(item["status"], "incomplete");
                assert_eq!(
                    response["incomplete_details"]["reason"],
                    if stop == 3 {
                        "max_output_tokens"
                    } else {
                        "content_filter"
                    },
                );
            } else {
                assert_eq!(events.len(), 7);
                assert_eq!(events[2]["type"], "response.output_item.added");
                assert_eq!(events[2]["item"][key], "");
                assert_eq!(events[2]["item"]["status"], "in_progress");
                assert_eq!(events[3]["type"], delta_event);
                assert_eq!(events[3]["delta"], payload);
                assert_eq!(events[3]["call_id"], "call-original");
                assert_eq!(events[4]["type"], done_event);
                assert_eq!(events[4][key], payload);
                assert_eq!(events[5]["type"], "response.output_item.done");
                assert_eq!(events[5]["item"], *item);
                for event in &events[2..6] {
                    assert_eq!(event["output_index"], 0);
                }
                for event in &events[3..5] {
                    assert_eq!(event["item_id"], item["id"]);
                }
                assert_eq!(events[6]["type"], "response.completed");
                assert_eq!(response["status"], "completed");
                assert_eq!(item["status"], "completed");
                assert!(response["incomplete_details"].is_null());
            }
        }
    }
}

#[test]
fn errors_and_limits_never_finalize_partial_calls() {
    let mut t = translator();
    t.push(call(false)).unwrap();
    let failed = t.failed_event("upstream_timeout", "Gateway timed out");
    assert_eq!(failed["response"]["output"], json!([]));
    assert_eq!(failed["response"]["error"]["code"], "upstream_timeout");
    assert_eq!(failed["type"], "response.failed");
    assert!(t.finish().is_err());
    let mut t = translator();
    t.push(call(false)).unwrap();
    t.push(ChatResponse {
        stop_reason: Some(3),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e["type"] == "response.output_item.done")
    );
    assert_eq!(events.last().unwrap()["type"], "response.incomplete");
}

#[test]
fn reject_unknown_disallowed_or_multiple_calls() {
    let mut t = Translator::new("model".into(), HashMap::new(), true, Mode::Strict);
    assert!(t.push(call(true)).is_err());
    let mut t = translator();
    t.allow_tools = false;
    assert!(t.push(call(true)).is_err());
    let mut t = translator();
    t.push(call(true)).unwrap();
    let mut second = call(true);
    second.delta_tool_calls[0].id = "b".into();
    assert!(t.push(second).is_err());
}

#[test]
fn encrypted_reasoning_keeps_native_identity_and_hides_raw_thought() {
    let mut t = translator();
    let signature = json!([reasoning()]).to_string();
    t.push(ChatResponse {
        delta_thinking: "PRIVATE".into(),
        delta_signature: signature[..10].into(),
        ..Default::default()
    })
    .unwrap();
    t.push(ChatResponse {
        delta_signature: signature[10..].into(),
        delta_signature_type: "openai".into(),
        delta_text: "answer".into(),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    assert_eq!(events.last().unwrap()["response"]["output"][0], reasoning());
    assert!(!serde_json::to_string(&events).unwrap().contains("PRIVATE"));
}

#[test]
fn multiple_reasoning_items_preserve_order_and_replay() {
    let first = reasoning();
    let mut second = reasoning();
    second["id"] = json!("rs_second");
    second["encrypted_content"] = json!("opaque-second");
    second["summary"] = json!([{"type":"summary_text","text":"summary"}]);
    let signature = json!([first, second]);
    for mode in [Mode::Strict, Mode::Codex] {
        for (text, tool) in [(true, false), (false, true), (true, true)] {
            if text && tool && mode == Mode::Strict {
                continue;
            }
            let mut t = translator();
            t.mode = mode;
            let mut events = t.created_events();
            if text {
                t.push(ChatResponse {
                    delta_text: "answer".into(),
                    ..Default::default()
                })
                .unwrap();
            }
            if tool {
                t.push(call(true)).unwrap();
            }
            let encoded = signature.to_string();
            for chunk in encoded.as_bytes().chunks(17) {
                t.push(ChatResponse {
                    delta_signature: String::from_utf8(chunk.to_vec()).unwrap(),
                    delta_signature_type: "openai".into(),
                    ..Default::default()
                })
                .unwrap();
            }
            events.extend(t.finish().unwrap());
            for (index, event) in events.iter().enumerate() {
                assert_eq!(event["sequence_number"], index);
            }
            let output = events.last().unwrap()["response"]["output"]
                .as_array()
                .unwrap();
            assert_eq!(output.len(), 2 + usize::from(text) + usize::from(tool));
            assert_eq!(&output[..2], signature.as_array().unwrap());
            let done: Vec<_> = events
                .iter()
                .filter(|event| event["type"] == "response.output_item.done")
                .collect();
            for (index, event) in done.iter().enumerate() {
                assert_eq!(event["output_index"], index);
                assert_eq!(event["item"], output[index]);
            }
            assert_eq!(done.len(), output.len());
            let mut input = output.clone();
            if tool {
                input.push(json!({"type":"custom_tool_call_output","call_id":"a","output":"ok"}));
            }
            input.push(json!({"role":"user","content":"continue"}));
            let mut replay = json!({"model":"gpt-6-astra","input":input});
            if mode == Mode::Codex {
                replay = crate::compat::Compatibility::prepare(&replay).unwrap().0;
            }
            let credentials = crate::config::Credentials {
                api_key: "test-key".into(),
                endpoint: "https://example.com/rpc".into(),
                device_id: "test-device".into(),
                client_version: "test".into(),
            };
            let translated = crate::request::translate_request(&replay, &credentials).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&translated.request.messages[0].reasoning).unwrap(),
                signature
            );
            assert_eq!(translated.request.messages[0].reasoning_type, "openai");
            assert!(
                translated.request.messages[1..]
                    .iter()
                    .all(|message| message.reasoning.is_empty())
            );
        }
    }
}

#[test]
fn reasoning_signature_rejects_empty_invalid_duplicate_and_oversized_arrays() {
    let mut invalid = reasoning();
    invalid["encrypted_content"] = json!("");
    for (signature, expected) in [
        (json!([]), "Empty reasoning signature cannot be replayed"),
        (
            json!([reasoning(), invalid]),
            "Invalid replayable reasoning identity",
        ),
        (
            json!([reasoning(), reasoning()]),
            "Gateway reused an output item ID",
        ),
        (
            json!(vec![reasoning(); MAX_ITEMS]),
            "Gateway response exceeds output item limit",
        ),
    ] {
        let mut t = translator();
        t.mode = Mode::Codex;
        t.push(ChatResponse {
            delta_signature: signature.to_string(),
            delta_signature_type: "openai".into(),
            delta_text: "PRIVATE".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(t.finish().unwrap_err().0, expected);
        let failure = t.failed_event("response_translation_error", expected);
        assert_eq!(failure["response"]["output"], json!([]));
        assert!(!failure.to_string().contains("PRIVATE"));
        assert!(t.finish().is_err());
    }
}

#[test]
fn reject_unreplayable_reasoning() {
    for patch in [
        json!({"encrypted_content":""}),
        json!({"unknown":true}),
        json!({"summary":[{"type":"summary_text","text":"x","annotations":[]}]}),
    ] {
        let mut item = reasoning();
        for (key, value) in patch.as_object().unwrap() {
            item[key] = value.clone();
        }
        let mut signature = ReasoningSignature::default();
        signature
            .push(&json!([item]).to_string(), "openai")
            .unwrap();
        assert!(signature.items(MAX_ITEMS).is_err());
    }
    let mut t = translator();
    t.push(ChatResponse {
        delta_signature: json!([reasoning()]).to_string(),
        delta_text: "answer".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(t.finish().is_err());
    let mut t = translator();
    t.push(ChatResponse {
        delta_text: "answer".into(),
        ..Default::default()
    })
    .unwrap();
    t.push(ChatResponse {
        delta_signature: json!([reasoning()]).to_string(),
        delta_signature_type: "openai".into(),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    assert_eq!(events.last().unwrap()["response"]["output"][0], reasoning());
    let replay =
        json!({"model":"gpt-6-astra","input":events.last().unwrap()["response"]["output"]});
    assert!(crate::request::audit_request(&replay).is_empty());
}

#[test]
fn raw_cache_counters_do_not_invent_total_usage() {
    let mut t = translator();
    t.push(ChatResponse {
        usage: Some(Usage {
            input_tokens: Some(100),
            output_tokens: Some(5),
            cache_read_tokens: Some(80),
            ..Default::default()
        }),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    let response = &events.last().unwrap()["response"];
    assert!(response["usage"].is_null());
    assert_eq!(response["x_devin_usage"]["cache_read_tokens"], 80);
}

#[test]
fn accumulated_private_thinking_is_bounded() {
    let mut t = translator();
    t.received_bytes = MAX_BYTES;
    assert!(
        t.push(ChatResponse {
            delta_thinking: "x".into(),
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn text_after_tool_stays_after_tool() {
    let mut t = translator();
    t.push(ChatResponse {
        delta_text: "before".into(),
        ..Default::default()
    })
    .unwrap();
    t.push(call(true)).unwrap();
    t.push(ChatResponse {
        delta_text: "after".into(),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    let output = events.last().unwrap()["response"]["output"]
        .as_array()
        .unwrap();
    assert_eq!(output.len(), 3);
    assert_eq!(output[0]["content"][0]["text"], "before");
    assert_eq!(output[1]["type"], "custom_tool_call");
    assert_eq!(output[2]["content"][0]["text"], "after");
}

#[test]
fn multi_item_reasoning_attachment_is_rejected() {
    let mut t = translator();
    t.push(ChatResponse {
        delta_text: "commentary".into(),
        ..Default::default()
    })
    .unwrap();
    t.push(call(true)).unwrap();
    t.push(ChatResponse {
        delta_signature: json!([reasoning()]).to_string(),
        delta_signature_type: "openai".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        t.finish().unwrap_err().0,
        "Reasoning attachment to multiple assistant items is unverified"
    );
}

#[test]
fn compatibility_attaches_signed_reasoning_before_assistant_turn() {
    let mut t = translator();
    t.mode = Mode::Codex;
    t.push(ChatResponse {
        delta_text: "commentary".into(),
        phase: "commentary".into(),
        ..Default::default()
    })
    .unwrap();
    t.push(call(true)).unwrap();
    t.push(ChatResponse {
        delta_signature: json!([reasoning()]).to_string(),
        delta_signature_type: "openai".into(),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    let output = &events.last().unwrap()["response"]["output"];
    assert_eq!(output[0], reasoning());
    assert_eq!(output[1]["phase"], "commentary");
    assert_eq!(output[2]["name"], "exec");
}

#[test]
fn reported_model_family_must_match() {
    for uid in ["different-model", "gpt-6-astral-medium"] {
        let mut t = translator();
        assert!(
            t.push(ChatResponse {
                actual_model_uid: uid.into(),
                ..Default::default()
            })
            .is_err()
        );
        assert!(t.finish().is_err());
    }
    let mut t = translator();
    t.push(ChatResponse {
        actual_model_uid: "gpt-6-astra-medium".into(),
        ..Default::default()
    })
    .unwrap();
    let events = t.finish().unwrap();
    assert_eq!(
        events.last().unwrap()["response"]["x_devin_usage"]["actual_model_uid"],
        "gpt-6-astra-medium"
    );
}
