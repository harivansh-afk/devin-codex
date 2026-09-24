use super::*;
use crate::{config::Credentials, request::translate_request};

fn request() -> Value {
    json!({"model":"gpt-6-astra", "reasoning":{"effort":"medium","context":"all_turns"},
    "text":{"verbosity":"low"}, "client_metadata":{}, "prompt_cache_key":"test", "service_tier":null,
    "input":[
        {"type":"additional_tools","role":"developer","tools":[
            {"type":"namespace","name":"functions","tools":[
                {"type":"custom","name":"exec","description":"Original description", "format":{"type":"grammar","syntax":"lark","definition":"start: /.+/"}}
            ]},
            {"type":"namespace","name":"other","tools":[
                {"type":"function","name":"exec","parameters":{"type":"object"},"strict":false}
            ]}
        ]},
        {"role":"developer","content":[{"type":"input_text","text":"First instruction"},{"type":"input_text","text":"Second instruction"}]},
        {"role":"user","content":"Do the task"},
        {"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"},
        {"type":"custom_tool_call","namespace":"functions","name":"exec","call_id":"c1","input":"text(42)"},
        {"type":"custom_tool_call_output","call_id":"c1","output":[{"type":"input_text","text":"42"}]}
    ]})
}

#[test]
fn lite_history_roundtrip_preserves_payloads_and_tool_identity() {
    let (body, mapping) = Compatibility::prepare(&request()).unwrap();
    let credentials = Credentials {
        api_key: "test".into(),
        endpoint: "https://example.invalid".into(),
        device_id: String::new(),
        client_version: String::new(),
    };
    let translated = translate_request(&body, &credentials).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        translated
            .request
            .system_prompt
            .contains("First instruction\n\nSecond instruction")
    );
    assert_eq!(translated.request.tools[0].name, "functions__exec");
    assert_eq!(
        translated.request.tools[0].description,
        "Original description"
    );
    assert_eq!(translated.request.tools[0].grammar, "start: /.+/");
    assert_eq!(translated.request.tools[1].name, "other__exec");
    assert_eq!(
        translated.request.messages[1].tool_calls[0].arguments_json,
        "text(42)"
    );
    assert!(translated.request.messages[1].reasoning.contains("opaque"));
    assert_eq!(translated.request.messages[2].content, "42");
    let call = json!({"type":"custom_tool_call","name":"functions__exec","call_id":"c2","input":"text(7)"});
    let mut event = json!({"type":"response.output_item.done","item":call});
    mapping.restore(&mut event);
    assert_eq!(event["item"]["namespace"], "functions");
    assert_eq!(event["item"]["name"], "exec");
    assert_eq!(event["item"]["input"], "text(7)");
    assert!(event.get("response").is_none());
    let mut final_event = json!({"type":"response.completed","response":{"output":[call]}});
    mapping.restore(&mut final_event);
    assert_eq!(final_event["response"]["output"][0], event["item"]);
}

#[test]
fn devin_model_alias_preserves_effort_and_response_identity() {
    let credentials = Credentials {
        api_key: "test".into(),
        endpoint: "https://example.invalid".into(),
        device_id: String::new(),
        client_version: String::new(),
    };
    for effort in ["low", "medium", "high", "xhigh", "max"] {
        let body =
            json!({"model":"gpt-6-astra-devin", "input":"hello", "reasoning":{"effort":effort}});
        let (prepared, mapping) = Compatibility::prepare(&body).unwrap();
        let translated = translate_request(&prepared, &credentials).unwrap();
        assert_eq!(translated.request.model, format!("gpt-6-astra-{effort}"));
        assert_eq!(translated.model, "gpt-6-astra");
        for kind in ["response.created", "response.completed", "response.failed"] {
            let mut event = json!({"type":kind,"response":{"model":"gpt-6-astra","output":[]}});
            mapping.restore(&mut event);
            assert_eq!(event["response"]["model"], "gpt-6-astra-devin");
        }
    }
    for model in ["other", "gpt-6-astra-devin-other"] {
        let (prepared, _) =
            Compatibility::prepare(&json!({"model":model,"input":"hello"})).unwrap();
        assert!(translate_request(&prepared, &credentials).is_err());
    }
}

#[test]
fn account_variant_routes_exact_uid_and_restores_alias() {
    let models = crate::models::Models::parse(&json!({"families":[
        {"slug":"gpt-6-sol","variants":[{"model_uid":"gpt-6-sol-medium"},{"model_uid":"gpt-6-sol-high"}]},
        {"slug":"claude-opus-5.5","variants":[{"model_uid":"claude-opus-5-5-high","max_output_tokens":8192}]},
        {"slug":"fusion","variants":[{"model_uid":"fusion-claude-fable-5-1-high-sidekick-swe-2-medium"}]}
    ]}))
    .unwrap();
    let credentials = Credentials {
        api_key: "test".into(),
        endpoint: "https://example.invalid".into(),
        device_id: String::new(),
        client_version: String::new(),
    };
    for (alias, effort, uid) in [
        ("devin-gpt-6-sol", "high", "gpt-6-sol-high"),
        ("devin-claude-opus-5-5", "high", "claude-opus-5-5-high"),
        (
            "devin-fable-5-1-fusion",
            "medium",
            "fusion-claude-fable-5-1-high-sidekick-swe-2-medium",
        ),
    ] {
        let (body, mapping) = Compatibility::prepare_with_models(
            &json!({"model":alias,"input":"hello","reasoning":{"effort":effort}}),
            &models,
        )
        .unwrap();
        let translated =
            crate::request::translate_with_models(&body, &credentials, &models).unwrap();
        assert_eq!(translated.request.model, uid);
        if uid == "claude-opus-5-5-high" {
            assert_eq!(
                translated.request.completion_config.unwrap().max_tokens,
                8192
            );
        }
        let mut event = json!({"type":"response.completed","response":{"model":body["model"]}});
        mapping.restore(&mut event);
        assert_eq!(event["response"]["model"], alias);
    }
    let (unknown, _) = Compatibility::prepare_with_models(
        &json!({"model":"devin-unknown","input":"hello"}),
        &models,
    )
    .unwrap();
    assert!(crate::request::translate_with_models(&unknown, &credentials, &models).is_err());
    assert!(
        !crate::request::audit_request(&json!({"model":"claude-opus-5-5","input":"hello"}))
            .is_empty()
    );
    let (unsupported, _) = Compatibility::prepare_with_models(
        &json!({"model":"devin-fable-5-1-fusion","input":"hello","reasoning":{"effort":"high"}}),
        &models,
    )
    .unwrap();
    assert!(crate::request::translate_with_models(&unsupported, &credentials, &models).is_err());
}

#[test]
fn collaboration_uses_codex_plaintext_dispatch_without_changing_other_tools() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        for namespace in ["collaboration", "other"] {
            let parameters = json!({"type":"object","properties":{"message":{"type":"string","description":"Task text","encrypted":true}},"required":["message"]});
            let body = json!({"model":MODEL,"input":"hello","tools":[{
                "type":"namespace","name":namespace,"tools":[{
                    "type":"function","name":name,"parameters":parameters
                }]
            }]});
            let (prepared, mapping) = Compatibility::prepare(&body).unwrap();
            let actual = &prepared["tools"][0]["parameters"];
            if namespace == "collaboration" {
                assert!(actual["properties"]["message"].get("encrypted").is_none());
                assert_eq!(actual["properties"]["message"]["description"], "Task text");
            } else {
                assert_eq!(actual, &parameters);
            }
            let call = json!({"type":"function_call","name":format!("{namespace}__{name}"),"call_id":"c1","arguments":"{\"message\":\"hello\"}"});
            let mut event = json!({"type":"response.output_item.done","item":call,"response":{"output":[call]}});
            mapping.restore(&mut event);
            let item = &event["item"];
            assert_eq!(item["namespace"], namespace);
            assert_eq!(item["name"], name);
            assert_eq!(item["arguments"], "{\"message\":\"hello\"}");
            assert_eq!(event["response"]["output"][0], *item);
            if namespace == "collaboration" {
                assert_eq!(item["encrypted_function_args"], json!([]));
                let mut followup = body.clone();
                followup["input"] =
                    json!([item, {"type":"function_call_output","call_id":"c1","output":"ok"}]);
                let (followup, _) = Compatibility::prepare(&followup).unwrap();
                assert!(crate::request::audit_request(&followup).is_empty());
            } else {
                assert!(item.get("encrypted_function_args").is_none());
            }
        }
    }
}

#[test]
fn reasoning_envelope_preserves_identity_when_clients_omit_ids() {
    let original =
        json!({"type":"reasoning","id":"rs_original","encrypted_content":"opaque","summary":[]});
    let (_, mapping) = Compatibility::prepare(&json!({"model":MODEL,"input":"hello"})).unwrap();
    let mut event = json!({"item":original,"response":{"output":[original]}});
    mapping.restore(&mut event);
    assert_eq!(event["item"], event["response"]["output"][0]);
    for omit_id in [false, true] {
        let mut replay = event["item"].clone();
        if omit_id {
            replay.as_object_mut().unwrap().remove("id");
        }
        let (prepared, _) = Compatibility::prepare(
            &json!({"model":MODEL,"input":[replay,{"role":"assistant","content":"done"}]}),
        )
        .unwrap();
        assert_eq!(prepared["input"][0], original);
    }
    for invalid in [
        json!(null),
        json!({"id":"rs_original"}),
        json!({"id":7,"encrypted_content":"opaque"}),
    ] {
        let mut replay = original.clone();
        replay["encrypted_content"] = json!(format!("{REASONING_PREFIX}{invalid}"));
        assert!(Compatibility::prepare(&json!({"model":MODEL,"input":[replay]})).is_err());
    }
    event["item"]["id"] = json!("rs_changed");
    assert!(Compatibility::prepare(&json!({"model":MODEL,"input":[event["item"]]})).is_err());
}

#[test]
fn interrupted_reasoning_keeps_an_empty_assistant_boundary() {
    let reasoning = json!({"type":"reasoning","encrypted_content":"opaque","summary":[]});
    for tail in [
        vec![],
        vec![json!({"role":"user","content":"interrupt"})],
        vec![
            json!({"type":"agent_message","author":"/root/child","recipient":"/root","content":[{"type":"input_text","text":"done"}]}),
        ],
    ] {
        let mut input = vec![reasoning.clone()];
        input.extend(tail);
        let (body, _) = Compatibility::prepare(&json!({"model":MODEL,"input":input})).unwrap();
        assert_eq!(body["input"][0], reasoning);
        assert_eq!(
            body["input"][1],
            json!({"type":"message","role":"assistant","content":""})
        );
        assert!(crate::request::audit_request(&body).is_empty());
    }
}

#[test]
fn compaction_keeps_historical_calls_without_readvertising_tools() {
    let body = json!({"model":MODEL,"tools":[],"tool_choice":"none","input":[
        {"type":"custom_tool_call","namespace":"functions","name":"exec","call_id":"c1","input":"text(42)"},
        {"type":"custom_tool_call_output","call_id":"c1","output":"42"},
        {"type":"function_call","namespace":"collaboration","name":"wait_agent","call_id":"c2","arguments":"{}"},
        {"type":"function_call_output","call_id":"c2","output":"done"}
    ]});
    let (prepared, _) = Compatibility::prepare(&body).unwrap();
    assert_eq!(prepared["tools"], json!([]));
    assert_eq!(prepared["input"][0]["name"], "functions__exec");
    assert_eq!(prepared["input"][0]["input"], "text(42)");
    assert!(crate::request::audit_request(&prepared).is_empty());
    let mut collision = body.clone();
    collision["tools"] = json!([{"type":"function","name":"functions__exec","parameters":{}}]);
    assert!(Compatibility::prepare(&collision).is_err());
}

#[test]
fn plaintext_agent_messages_translate_but_encrypted_messages_fail() {
    let mut body = json!({"model":"gpt-6-astra","input":[{
        "type":"agent_message","author":"/root/probe","recipient":"/root",
        "content":[{"type":"input_text","text":"Message Type: FINAL_ANSWER\nPayload:\nCHILD_OK"}]
    }]});
    let (prepared, _) = Compatibility::prepare(&body).unwrap();
    assert_eq!(prepared["input"][0]["role"], "user");
    let content = prepared["input"][0]["content"].as_str().unwrap();
    assert!(content.contains("/root/probe"));
    assert!(content.contains("CHILD_OK"));
    assert!(crate::request::audit_request(&prepared).is_empty());
    body["input"][0]["content"] =
        json!([{"type":"encrypted_content","encrypted_content":"opaque"}]);
    assert!(Compatibility::prepare(&body).is_err());
}

#[test]
fn unsupported_content_and_name_collisions_fail_before_inference() {
    let mut body = request();
    body["input"][2]["content"] = json!([{"type":"input_image","image_url":"data:private"}]);
    let (prepared, _) = Compatibility::prepare(&body).unwrap();
    assert!(!crate::request::audit_request(&prepared).is_empty());
    let mut body = request();
    body["tools"] = json!([{"type":"function","name":"functions__exec","parameters":{}}]);
    assert!(Compatibility::prepare(&body).is_err());
    for body in [
        Value::Null,
        json!({"input":null}),
        json!({"input":[1]}),
        json!({"input":"hello","tools":[null]}),
    ] {
        if let Ok((prepared, _)) = Compatibility::prepare(&body) {
            assert!(!crate::request::audit_request(&prepared).is_empty());
        }
    }
    let mut body = request();
    body["tools"] = json!([{"type":"web_search"}]);
    assert!(Compatibility::prepare(&body).is_err());
}
