use baton_core::project::{ProjectConfig, parse_config};

fn example() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 3,
        "agents":[{"id":"writer", "systemPrompt":"Write clearly", "rules":[], "model":{"provider":"example", "model":"large", "maxOutputTokens":1024}, "toolIds":["read"], "skillIds":["style"]}],
        "pipelines":[{"id":"draft", "name":"Draft", "steps":[
            {"id":"plan", "name":"Plan", "agentId":"writer"},
            {"id":"build", "name":"Build", "agentId":"writer"},
            {"id":"review", "name":"Review", "agentId":"writer"}
        ]}],
        "tools":[{"id":"read", "name":"Read"}],
        "skills":[{"id":"style", "name":"Style", "instructions":"Be concise"}]
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
            "/agents/0/model/maxOutputTokens",
            serde_json::json!(0),
            "maxOutputTokens",
        ),
        (
            "/agents/0/toolIds",
            serde_json::json!(["read", "read"]),
            "duplicate ID",
        ),
        (
            "/agents/0/skillIds",
            serde_json::json!(["missing"]),
            "skillIds: unknown reference",
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
    ] {
        let mut value = example();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let error = parse_config(&value.to_string()).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    for collection in ["agents", "pipelines", "tools", "skills"] {
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
