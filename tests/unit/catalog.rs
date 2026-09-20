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
    Catalog::prepare(&mut value).unwrap();
    assert_eq!(value["models"].as_array().unwrap().len(), 1);
    let model = &value["models"][0];
    assert_eq!(model["slug"], crate::compat::MODEL);
    let efforts: Vec<_> = model["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|level| level["effort"].as_str().unwrap())
        .collect();
    assert_eq!(efforts, crate::compat::EFFORTS);
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
    assert_eq!(model["input_modalities"], json!(["text"]));
    for mut invalid in [
        json!({}),
        json!({"models":[]}),
        json!({"models":[{"slug":"gpt-6-astra","supported_reasoning_levels":[]}]}),
    ] {
        assert!(Catalog::prepare(&mut invalid).is_err());
    }
}
