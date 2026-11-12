use baton_core::domain::project::{Agent, parse_config};

#[test]
fn agent_provider_and_model_round_trip() {
    let agent: Agent = serde_json::from_value(serde_json::json!({
        "id": "writer", "instructions": "Write", "provider": "rig",
        "model": "openai/family/model", "credentialId": "credential-1", "skillIds": []
    }))
    .unwrap();
    let saved = serde_json::to_value(&agent).unwrap();
    assert_eq!(saved["provider"], "rig");
    assert_eq!(saved["model"], "openai/family/model");
    assert_eq!(serde_json::from_value::<Agent>(saved).unwrap(), agent);
}

#[test]
fn old_runtime_and_model_formats_are_rejected() {
    for agent in [
        serde_json::json!({"id":"a", "instructions":"x", "runtimeKind":"rig", "skillIds":[]}),
        serde_json::json!({"id":"a", "instructions":"x", "provider":"rig", "model":{"provider":"openai", "model":"gpt"}, "skillIds":[]}),
        serde_json::json!({"id":"a", "instructions":"x", "provider":"rig", "model":"invalid", "skillIds":[]}),
    ] {
        assert!(serde_json::from_value::<Agent>(agent).is_err());
    }
    assert!(
        parse_config(
            &serde_json::json!({"schemaVersion":6,"agents":[],"pipelines":[],"runtimeProfiles":[]})
                .to_string()
        )
        .is_err()
    );
    let run = serde_json::json!({
        "id":"run", "pipelineId":"p", "task":"t", "startedAt":"now", "endedAt":null,
        "durationMs":null, "status":"failed", "steps":[],
        "configSnapshot":{"schemaVersion":7,"agents":[{"id":"a","instructions":"x","runtimeKind":"rig","skillIds":[]}],"pipelines":[]}
    });
    assert!(serde_json::from_value::<baton_core::domain::pipeline::PipelineRun>(run).is_err());
}
