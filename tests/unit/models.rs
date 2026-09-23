use super::*;
use serde_json::json;

#[test]
fn only_curated_families_and_fable_fusion_are_exposed() {
    let listing = json!({"families":[
        {"slug":"gpt-6-sol","variants":[
            {"model_uid":"gpt-6-sol-medium","max_context_tokens":1000000},
            {"model_uid":"gpt-6-sol-high"},
            {"model_uid":"gpt-6-sol-high-priority"}]},
        {"slug":"gpt-6-astra","variants":[{"model_uid":"gpt-6-astra-medium"}]},
        {"slug":"claude-opus-5.5","variants":[
            {"model_uid":"claude-opus-5-5-high","max_output_tokens":8192}]},
        {"slug":"fusion","variants":[
            {"model_uid":"fusion-claude-fable-5-1-high-sidekick-swe-2-medium"},
            {"model_uid":"fusion-claude-fable-5-1-low-sidekick-swe-2-medium"}]},
        {"slug":"gpt-5.5","variants":[{"model_uid":"gpt-5-5-high"}]}
    ]});
    let models = Models::parse(&listing).unwrap();
    assert_eq!(models.entries().len(), 4);
    assert_eq!(
        models
            .entries()
            .iter()
            .map(|m| m.alias.as_str())
            .collect::<Vec<_>>(),
        [
            "devin-gpt-6-sol",
            "gpt-6-astra-devin",
            "devin-claude-opus-5-5",
            "devin-fable-5-1-fusion"
        ]
    );
    let sol = models.model("gpt-6-sol").unwrap();
    assert_eq!(sol.context, 1000000);
    assert_eq!(sol.variants.len(), 2);
    assert_eq!(sol.default_effort(), "medium");
    assert_eq!(
        models.model("claude-opus-5-5").unwrap().default_effort(),
        "high"
    );
    assert_eq!(
        models.model("claude-opus-5-5").unwrap().variants["high"].output,
        8192
    );
    assert_eq!(
        models.model("fable-5-1-fusion").unwrap().variants["medium"].uid,
        "fusion-claude-fable-5-1-high-sidekick-swe-2-medium"
    );
    assert!(models.model("gpt-5.5").is_none());
    assert!(models.model("devin-unknown").is_none());
    assert!(Models::parse(&json!({"families":[]})).is_err());
}
