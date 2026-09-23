use super::*;
use serde_json::json;

#[test]
fn parses_account_variants_and_rejects_ambiguous_uids() {
    let listing = json!({"families":[
        {"slug":"claude-opus-5.5","variants":[
            {"model_uid":"claude-opus-5-5-medium","label":"Claude Opus Medium",
             "max_context_tokens":1000000,"max_output_tokens":128000},
            {"model_uid":"MODEL_PRIVATE_11","label":"Legacy Claude",
             "max_context_tokens":200000,"max_output_tokens":64000}
        ]},
        {"slug":"fusion","variants":[
            {"model_uid":"fusion-claude-sidekick-swe","label":"Fusion"}
        ]}
    ]});
    let models = Models::parse(&listing).unwrap();
    assert_eq!(models.entries().len(), 3);
    assert_eq!(
        models.uid("devin-MODEL_PRIVATE_11"),
        Some("MODEL_PRIVATE_11")
    );
    assert_eq!(models.entries()[0].context, 1000000);
    assert!(models.contains_uid("fusion-claude-sidekick-swe"));
    assert!(!models.contains_uid("unknown"));
    let mut duplicate = listing.clone();
    duplicate["families"][1]["variants"][0]["model_uid"] = json!("MODEL_PRIVATE_11");
    assert!(Models::parse(&duplicate).is_err());
    duplicate["families"][1]["variants"][0]["model_uid"] = json!("../bad");
    assert!(Models::parse(&duplicate).is_err());
    assert!(Models::parse(&json!({"families":[]})).is_err());
}
