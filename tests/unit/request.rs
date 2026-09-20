use super::*;
use serde_json::json;

fn creds() -> Credentials {
    Credentials {
        api_key: "test-key".into(),
        endpoint: "https://example.invalid".into(),
        device_id: "test-device".into(),
        client_version: "3000.10.31".into(),
    }
}
fn base() -> Value {
    json!({"model":"gpt-6-astra", "input":"hello"})
}
fn history() -> Value {
    json!({"model":"gpt-6-astra", "tools":[{"type":"function", "name":"run", "parameters":{"type":"object"}}], "input":[
        {"type":"function_call", "call_id":"c1", "name":"run", "arguments":"{\n}"},
        {"type":"function_call_output", "call_id":"c1", "output":"done"}
    ]})
}
#[test]
fn minimal_and_history_translate() {
    for body in [base(), history()] {
        assert!(audit_request(&body).is_empty());
        assert!(translate_request(&body, &creds()).is_ok());
    }
    let t = translate_request(&history(), &creds()).ok().unwrap();
    assert_eq!(t.request.messages[0].tool_calls[0].arguments_json, "{\n}");
    assert_eq!(t.request.messages[1].tool_call_id, "c1");
    assert_eq!(t.request.model, "gpt-6-astra-medium");
}
#[test]
fn completed_history_does_not_require_current_tool_definitions() {
    let mut body = history();
    body["tools"] = json!([]);
    body["tool_choice"] = json!("none");
    let translated = translate_request(&body, &creds()).unwrap();
    assert!(translated.request.tools.is_empty());
    assert!(translated.tool_map.is_empty());
    assert!(!translated.allow_tools);
    assert_eq!(translated.request.messages[0].tool_calls[0].name, "run");
    assert_eq!(translated.request.messages[1].content, "done");
    for invalid in ["", "namespace.run", "with spaces"] {
        body["input"][0]["name"] = json!(invalid);
        assert!(translate_request(&body, &creds()).is_err());
    }
    let mut body = history();
    body["input"][0]["type"] = json!("custom_tool_call");
    assert!(translate_request(&body, &creds()).is_err());
}
#[test]
fn invalid_json_types_never_panic() {
    for value in [Value::Null, json!([]), json!({}), json!(1), json!(true)] {
        for target in [
            "effort",
            "name",
            "call_id",
            "result_id",
            "item_id",
            "status",
            "type",
        ] {
            let mut body = history();
            match target {
                "effort" => body["reasoning"] = json!({"effort":value}),
                "result_id" => body["input"][1]["call_id"] = value.clone(),
                "item_id" => body["input"][0]["id"] = value.clone(),
                key => body["input"][0][key] = value.clone(),
            }
            assert!(
                translate_request(&body, &creds()).is_err(),
                "{target}: {value}"
            );
        }
    }
}
#[test]
fn reasoning_and_phase_preserved() {
    let reasoning = json!({"type":"reasoning", "id":"r1", "encrypted_content":"opaque", "status":"", "content":[], "summary":[{"type":"summary_text", "text":"summary"}]});
    let mut body = base();
    body["input"] = json!([reasoning, {"role":"assistant", "content":[{"type":"output_text", "text":"hello", "annotations":[], "logprobs":[]}], "phase":"commentary"}]);
    let t = translate_request(&body, &creds()).ok().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&t.request.messages[0].reasoning).unwrap(),
        json!([reasoning])
    );
    assert_eq!(t.request.messages[0].reasoning_type, "openai");
    assert_eq!(t.request.messages[0].phase, "commentary");
    body["input"][0]["content"] = Value::Null;
    let t = translate_request(&body, &creds()).unwrap();
    let replay: Value = serde_json::from_str(&t.request.messages[0].reasoning).unwrap();
    assert!(replay[0]["content"].is_null());
    assert_eq!(replay[0]["encrypted_content"], "opaque");
}
#[test]
fn grammar_and_false_strict_preserved() {
    let mut body = history();
    body["tools"][0]["strict"] = json!(false);
    body["tools"].as_array_mut().unwrap().push(json!({"type":"custom", "name":"patch", "description":"Original\nDescription", "format":{"type":"grammar", "syntax":"lark", "definition":"start: \"ok\"\n"}}));
    let t = translate_request(&body, &creds()).ok().unwrap();
    assert_eq!(t.request.tools[0].strict, Some(false));
    assert_eq!(t.request.tools[1].grammar, "start: \"ok\"\n");
    assert_eq!(t.request.tools[1].description, "Original\nDescription");
    assert_eq!(t.tool_map["patch"], ToolKind::Custom);
}
#[test]
fn unsupported_features_fail_closed() {
    for item in [
        json!({"role":"developer", "content":"keep exact"}),
        json!({"role":"system", "content":"keep exact"}),
        json!({"type":"additional_tools", "role":"developer", "tools":[]}),
        json!({"role":"assistant", "content":[{"type":"output_text", "text":"x", "annotations":{}}]}),
        json!({"role":"user", "content":[{"type":"input_text", "text":"x", "annotations":[]}]}),
        json!({"role":"user", "content":"x", "phase":"commentary"}),
        json!({"role":"assistant", "content":"x", "unknown":false}),
        json!({"type":"reasoning", "encrypted_content":"opaque", "summary":[{"type":"summary_text", "text":"x", "other":0}]}),
    ] {
        let mut body = base();
        body["input"] = json!([item]);
        assert!(!audit_request(&body).is_empty(), "{item}");
    }
    let mut body = base();
    body["tools"] = json!([{"type":"namespace", "name":"functions", "tools":[]}]);
    assert!(!audit_request(&body).is_empty());
    body = base();
    body["future_field"] = json!(false);
    assert!(!audit_request(&body).is_empty());
}
#[test]
fn tool_history_integrity_checked() {
    for case in [
        "kind",
        "duplicate_output",
        "orphan",
        "duplicate_call",
        "unanswered",
    ] {
        let mut body = history();
        match case {
            "kind" => body["input"][1]["type"] = json!("custom_tool_call_output"),
            "duplicate_output" => {
                let x = body["input"][1].clone();
                body["input"].as_array_mut().unwrap().push(x);
            }
            "orphan" => body["input"][1]["call_id"] = json!("other"),
            "duplicate_call" => {
                let calls = body["input"].as_array().unwrap().clone();
                body["input"].as_array_mut().unwrap().extend(calls);
            }
            "unanswered" => {
                body["input"].as_array_mut().unwrap().pop();
            }
            _ => unreachable!(),
        }
        assert!(!audit_request(&body).is_empty(), "{case}");
    }
}
#[test]
fn reasoning_never_attaches_to_nonassistant() {
    let r = json!({"type":"reasoning", "encrypted_content":"opaque", "summary":[]});
    let mut body = history();
    body["input"].as_array_mut().unwrap().insert(1, r.clone());
    assert!(!audit_request(&body).is_empty());
    body["input"] = json!([r, {"role":"user", "content":"hello"}]);
    assert!(!audit_request(&body).is_empty());
}
#[test]
fn ranges_and_no_tools_choice() {
    for (key, value) in [
        ("temperature", json!(0)),
        ("top_p", json!(2)),
        ("max_output_tokens", json!(true)),
        ("max_output_tokens", json!(128001)),
        ("max_output_tokens", json!(1.0)),
        ("store", json!(true)),
        ("parallel_tool_calls", json!(true)),
    ] {
        let mut b = base();
        b[key] = value;
        assert!(!audit_request(&b).is_empty(), "{key}");
    }
    let mut b = base();
    b["tool_choice"] = json!("none");
    let t = translate_request(&b, &creds()).ok().unwrap();
    assert!(!t.allow_tools);
    assert_eq!(t.request.tool_choice.unwrap().mode, "none");
}
