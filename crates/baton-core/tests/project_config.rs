use baton_core::domain::project::{ProjectConfig, parse_config};

fn example() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 7,
        "agents":[{"id":"writer", "systemPrompt":"Write clearly", "runtimeKind":"rig", "model":{"provider":"openai","model":"gpt-4.1"}, "credentialId":null, "rules":[], "skillIds":["style"]}],
        "pipelines":[{"id":"draft", "name":"Draft", "steps":[
            {"id":"plan", "name":"Plan", "agentId":"writer"},
            {"id":"build", "name":"Build", "agentId":"writer"},
            {"id":"review", "name":"Review", "agentId":"writer"}
        ]}]
    })
}

#[test]
fn migrates_only_exact_legacy_marker() {
    assert_eq!(
        parse_config(r#"{"schemaVersion":1}"#).unwrap(),
        ProjectConfig::CURRENT
    );
    for input in [
        r#"{"schemaVersion":1,"secret":"x"}"#,
        r#"{"schemaVersion":0}"#,
        r#"{"schemaVersion":99}"#,
        "{}",
        "not json",
    ] {
        assert!(parse_config(input).is_err(), "{input}");
    }
}

#[test]
fn demo_commit_agent_pipeline_is_valid_and_commits_before_human_approval() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/demo/.baton/config.json");
    let source = std::fs::read_to_string(path).unwrap();
    let config = parse_config(&source).unwrap();
    let pipeline = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == "draft")
        .unwrap();
    let commit = pipeline
        .steps
        .iter()
        .find(|step| step.id == "commit-message")
        .unwrap();
    assert_eq!(commit.agent_id, "commit-agent");
    assert_eq!(commit.transitions[0].to.as_deref(), Some("human-decision"));
    let review = pipeline
        .steps
        .iter()
        .find(|step| step.id == "review")
        .unwrap();
    assert!(review.transitions.iter().any(|transition| {
        transition.outcome == baton_core::domain::project::Outcome::Approved
            && transition.to.as_deref() == Some("commit-message")
    }));
}

#[test]
fn roundtrip_preserves_pipeline_step_order_and_configuration() {
    let config = parse_config(&example().to_string()).unwrap();
    assert_eq!(
        parse_config(&serde_json::to_string(&config).unwrap()).unwrap(),
        config
    );
    assert_eq!(config.pipelines[0].steps[2].id, "review");
}

#[test]
fn actionable_validation_failures() {
    for (pointer, replacement, message) in [
        (
            "/pipelines/0/steps/0/id",
            serde_json::json!("bad id"),
            "invalid ID",
        ),
        (
            "/pipelines/0/steps/0/name",
            serde_json::json!(" "),
            "name must not be empty",
        ),
        (
            "/agents/0/model/provider",
            serde_json::json!(" "),
            "invalid provider or model",
        ),
        (
            "/agents/0/skillIds",
            serde_json::json!(["bad id"]),
            "skillIds: invalid ID",
        ),
        (
            "/pipelines/0/steps/0/agentId",
            serde_json::json!("missing"),
            "agentId: unknown reference",
        ),
        (
            "/agents/0/systemPrompt",
            serde_json::json!(" "),
            "systemPrompt must not be empty",
        ),
        (
            "/pipelines/0/steps/1/id",
            serde_json::json!("plan"),
            "duplicate ID",
        ),
        (
            "/pipelines/0/steps",
            serde_json::json!([]),
            "at least one step",
        ),
        (
            "/schemaVersion",
            serde_json::json!(99),
            "Unsupported schemaVersion",
        ),
        (
            "/schemaVersion",
            serde_json::json!(5),
            "Pi runtime profiles are no longer supported",
        ),
    ] {
        let mut value = example();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let error = parse_config(&value.to_string()).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    for collection in ["agents", "pipelines"] {
        let mut value = example();
        let items = value[collection].as_array_mut().unwrap();
        items.push(items[0].clone());
        assert!(
            parse_config(&value.to_string())
                .unwrap_err()
                .contains("duplicate ID")
        );
    }
    let mut value = example();
    value["pipelines"][0]["steps"][0]["apiKey"] = serde_json::json!("not allowed");
    assert!(
        parse_config(&value.to_string())
            .unwrap_err()
            .contains("apiKey")
    );
}
