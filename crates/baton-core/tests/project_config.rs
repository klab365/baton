use baton_core::project::{ProjectConfig, parse_config};

fn example() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 2,
        "pipelines":[{"id":"draft", "name":"Draft", "steps":[
            {"id":"plan", "name":"Plan", "model":{"provider":"example", "model":"large", "maxOutputTokens":1024}, "toolIds":["read"], "skillIds":["style"], "execution":{"mode":"local", "command":["writer"]}},
            {"id":"build", "name":"Build", "model":{"provider":"example", "model":"large"}, "toolIds":[], "skillIds":[], "execution":{"mode":"docker", "image":"writer:latest", "command":["writer"], "workingDirectory":"/workspace"}},
            {"id":"review", "name":"Review", "model":{"provider":"example", "model":"large"}, "toolIds":[], "skillIds":[], "execution":{"mode":"remote", "host":"build.example", "command":["writer"]}}
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
            "/pipelines/0/steps/0/model/maxOutputTokens",
            serde_json::json!(0),
            "maxOutputTokens",
        ),
        (
            "/pipelines/0/steps/0/toolIds",
            serde_json::json!(["read", "read"]),
            "duplicate ID",
        ),
        (
            "/pipelines/0/steps/0/skillIds",
            serde_json::json!(["missing"]),
            "skillIds: unknown reference",
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
            "/pipelines/0/steps/0/execution/command",
            serde_json::json!([]),
            "command must contain",
        ),
        (
            "/pipelines/0/steps/1/execution/image",
            serde_json::json!(""),
            "image must not be empty",
        ),
        (
            "/pipelines/0/steps/2/execution/host",
            serde_json::json!(""),
            "host must not be empty",
        ),
        (
            "/schemaVersion",
            serde_json::json!(3),
            "Unsupported schemaVersion",
        ),
    ] {
        let mut value = example();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let error = parse_config(&value.to_string()).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    for collection in ["pipelines", "tools", "skills"] {
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
