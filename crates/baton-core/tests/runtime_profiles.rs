use baton_core::domain::{
    project::{ProjectConfig, parse_config},
    runtime_profiles::{self, RuntimeKind},
};

fn config() -> serde_json::Value {
    serde_json::from_str(include_str!("fixture_config.json")).unwrap()
}

#[test]
fn legacy_agents_migrate_to_rig_never_to_old_provider_or_fake() {
    for version in [2, 3] {
        let mut value = config();
        value["schemaVersion"] = version.into();
        for agent in value["agents"].as_array_mut().unwrap() {
            agent.as_object_mut().unwrap().remove("runtimeProfileId");
            agent["model"] =
                serde_json::json!({"provider":"fake", "model":"legacy", "maxOutputTokens":1024});
        }
        let migrated = parse_config(&value.to_string()).unwrap();
        assert_eq!(migrated.schema_version, 6);
        assert!(
            migrated
                .agents
                .iter()
                .all(|a| a.runtime_profile_id == "rig.default")
        );
        let task: baton_core::domain::tasks::Task = serde_json::from_value(serde_json::json!({
            "id":"task", "pipelineId":"draft", "configSnapshot":value, "status":"draft", "createdAt":"now", "updatedAt":"now"
        })).unwrap();
        assert_eq!(task.pipeline_id, "draft");
        assert!(
            serde_json::to_value(task)
                .unwrap()
                .get("configSnapshot")
                .is_none()
        );
        let run: baton_core::domain::pipeline::PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"run", "pipelineId":"draft", "task":"task", "startedAt":"now", "endedAt":null, "durationMs":null, "status":"failed", "steps":[], "configSnapshot":value
        })).unwrap();
        assert_eq!(run.config_snapshot.as_ref(), Some(&migrated));
        let encoded = serde_json::to_string(&migrated).unwrap();
        assert!(!encoded.contains("provider"));
        assert!(!encoded.contains("maxOutputTokens"));
        value["agents"][0]["model"]["apiKey"] = "do-not-copy".into();
        let error = parse_config(&value.to_string()).unwrap_err();
        assert!(error.contains("Unsupported legacy"));
        assert!(!error.contains("do-not-copy"));
    }
}

#[test]
fn assignment_requires_explicit_profile_and_steps_cannot_choose_runtime() {
    let mut value = config();
    assert_eq!(
        runtime_profiles::resolve(
            &parse_config(&value.to_string()).unwrap(),
            "rig.openai.gpt-4.1"
        )
        .unwrap()
        .runtime_kind,
        RuntimeKind::Rig
    );
    value["agents"][0]["runtimeProfileId"] = "fake.default".into();
    assert!(
        parse_config(&value.to_string())
            .unwrap_err()
            .contains("Unknown runtime profile")
    );
    value["runtimeProfiles"] = serde_json::json!([{"id":"fake.default", "name":"Offline tests", "runtimeKind":"fake", "model":null}]);
    value["agents"][1]["runtimeProfileId"] = "fake.default".into();
    let configured = parse_config(&value.to_string()).unwrap();
    assert_eq!(
        runtime_profiles::resolve(&configured, "fake.default")
            .unwrap()
            .runtime_kind,
        RuntimeKind::Fake
    );
    assert!(runtime_profiles::resolve(&configured, "fake").is_err());
    assert!(runtime_profiles::resolve(&ProjectConfig::CURRENT, "fake.default").is_err());
    for field in ["runtime", "runtimeProfileId", "provider", "model"] {
        let mut invalid = value.clone();
        invalid["pipelines"][0]["steps"][0][field] = "forbidden".into();
        assert!(parse_config(&invalid.to_string()).is_err(), "{field}");
    }
    for kind in ["rig", "openai", "chatgpt"] {
        let mut invalid = value.clone();
        invalid["runtimeProfiles"][0]["runtimeKind"] = kind.into();
        assert!(parse_config(&invalid.to_string()).is_err());
    }
    value["runtimeProfiles"][0]["id"] = "rig.local".into();
    assert!(parse_config(&value.to_string()).is_err());
}
