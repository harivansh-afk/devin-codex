use super::*;

#[test]
fn catalog_preserves_harness_and_only_advertises_supported_options() {
    let original = json!({
        "slug":"gpt-6-astra", "default_reasoning_level":"medium",
        "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"},{"effort":"max"},{"effort":"ultra"}],
        "base_instructions":"unchanged", "model_messages":{"instructions_template":"unchanged"},
        "tool_mode":"code_mode_only", "multi_agent_version":"v2", "use_responses_lite":true,
        "context_window":272000, "multi_agent_reasoning_effort":"xhigh",
        "service_tiers":[{"id":"priority"}], "additional_speed_tiers":["fast"], "input_modalities":["text","image"]
    });
    let mut value = json!({"models":[original,{"slug":"other"}]});
    let available = crate::models::Models::parse(&json!({"families":[
        {"slug":"gpt-6-sol","variants":[{"model_uid":"gpt-6-sol-medium"},{"model_uid":"gpt-6-sol-high"}]},
        {"slug":"gpt-6-astra","variants":[{"model_uid":"gpt-6-astra-medium"}]},
        {"slug":"claude-opus-5.5","variants":[{"model_uid":"claude-opus-5-5-high","max_context_tokens":1000000}]},
        {"slug":"fusion","variants":[{"model_uid":"fusion-claude-fable-5-1-high-sidekick-swe-2-medium"}]}
    ]}))
    .unwrap();
    Catalog::prepare(&mut value, &available).unwrap();
    assert_eq!(value["models"].as_array().unwrap().len(), 4);
    let model = &value["models"][1];
    assert_eq!(model["slug"], crate::compat::MODEL);
    let efforts: Vec<_> = model["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|level| level["effort"].as_str().unwrap())
        .collect();
    assert_eq!(efforts, ["medium"]);
    for key in [
        "base_instructions",
        "model_messages",
        "tool_mode",
        "multi_agent_version",
        "use_responses_lite",
        "context_window",
        "multi_agent_reasoning_effort",
        "default_reasoning_level",
    ] {
        assert_eq!(model[key], original[key], "{key}");
    }
    assert_eq!(model["service_tiers"], json!([]));
    assert_eq!(model["additional_speed_tiers"], json!([]));
    assert_eq!(model["input_modalities"], json!(["text", "image"]));
    let sol = &value["models"][0];
    assert_eq!(sol["slug"], "devin-gpt-6-sol");
    assert_eq!(
        sol["supported_reasoning_levels"].as_array().unwrap().len(),
        2
    );
    let variant = &value["models"][2];
    assert_eq!(variant["slug"], "devin-claude-opus-5-5");
    assert_eq!(variant["display_name"], "Claude Opus 5.5 (Devin)");
    assert_eq!(variant["context_window"], 1000000);
    assert_eq!(variant["supported_reasoning_levels"][0]["effort"], "high");
    assert_eq!(variant["tool_mode"], original["tool_mode"]);
    assert_eq!(value["models"][3]["slug"], "devin-fable-5-1-fusion");
    let without_astra =
        crate::models::Models::parse(&json!({"families":[{"slug":"claude-opus-5.5","variants":[
            {"model_uid":"claude-opus-5-5-high"}
        ]}]}))
        .unwrap();
    let mut other = json!({"models":[original]});
    Catalog::prepare(&mut other, &without_astra).unwrap();
    assert_eq!(other["models"].as_array().unwrap().len(), 1);
    assert_eq!(other["models"][0]["slug"], "devin-claude-opus-5-5");
    for mut invalid in [
        json!({}),
        json!({"models":[]}),
        json!({"models":[{"slug":"gpt-6-astra","supported_reasoning_levels":[]}]}),
    ] {
        assert!(Catalog::prepare(&mut invalid, &available).is_err());
    }
}
