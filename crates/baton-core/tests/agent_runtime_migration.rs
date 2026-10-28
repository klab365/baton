use baton_core::domain::{project::parse_config, runtime::RuntimeKind};

#[test]
fn schema_six_runtime_profiles_migrate_directly_to_agents() {
    let legacy = serde_json::json!({
        "schemaVersion": 6,
        "agents": [
            {"id":"writer","systemPrompt":"Write","runtimeProfileId":"rig.openai.gpt-4.1","rules":[],"skillIds":[]},
            {"id":"reviewer","systemPrompt":"Review","runtimeProfileId":"fake.default","rules":[],"skillIds":[]}
        ],
        "runtimeProfiles": [
            {"id":"rig.openai.gpt-4.1","name":"OpenAI","runtimeKind":"rig","model":{"provider":"openai","model":"gpt-4.1"},"credentialId":"credential-1"},
            {"id":"fake.default","name":"Offline","runtimeKind":"fake","model":null}
        ],
        "pipelines":[],"skills":[]
    });
    let config = parse_config(&legacy.to_string()).unwrap();
    assert_eq!(config.schema_version, 7);
    assert!(config.agents[0].runtime_kind == RuntimeKind::Rig);
    assert_eq!(config.agents[0].model.as_ref().unwrap().provider, "openai");
    assert_eq!(
        config.agents[0].credential_id.as_deref(),
        Some("credential-1")
    );
    assert!(config.agents[1].runtime_kind == RuntimeKind::Fake);
    let serialized = serde_json::to_value(config).unwrap();
    assert!(serialized.get("runtimeProfiles").is_none());
    assert!(serialized["agents"][0].get("runtimeProfileId").is_none());
}

#[test]
fn legacy_v2_v3_agents_migrate_without_secret_material() {
    for version in [2, 3] {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("fixture_config.json")).unwrap();
        value["schemaVersion"] = version.into();
        for agent in value["agents"].as_array_mut().unwrap() {
            let object = agent.as_object_mut().unwrap();
            object.remove("runtimeKind");
            object.remove("model");
            object.remove("credentialId");
            object.remove("runtimeProfileId");
            object.insert(
                "model".into(),
                serde_json::json!({"provider":"legacy","model":"old"}),
            );
        }
        let config = parse_config(&value.to_string()).unwrap();
        assert_eq!(config.schema_version, 7);
        assert!(
            config
                .agents
                .iter()
                .all(|agent| agent.runtime_kind == RuntimeKind::Rig)
        );
        let encoded = serde_json::to_string(&config).unwrap();
        assert!(!encoded.contains("runtimeProfile"));
        value["agents"][0]["model"]["apiKey"] = "must-not-leak".into();
        let error = parse_config(&value.to_string()).unwrap_err();
        assert!(error.contains("Unsupported legacy"));
        assert!(!error.contains("must-not-leak"));
    }
}
