use baton_core::domain::project::{ProjectConfig, parse_config};

fn example() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 7,
        "agents":[{"id":"writer", "systemPrompt":"Write clearly", "provider":"rig", "model":"openai/gpt-4.1", "credentialId":null, "rules":[], "skillIds":["style"]}],
        "pipelines":[{"id":"draft", "name":"Draft", "steps":[
            {"id":"plan", "name":"Plan", "type":"agent", "agentId":"writer"},
            {"id":"build", "name":"Build", "type":"agent", "agentId":"writer"},
            {"id":"review", "name":"Review", "type":"agent", "agentId":"writer"}
        ]}]
    })
}

#[test]
fn legacy_pipeline_names_are_accepted_but_not_serialized() {
    use baton_core::domain::project::Pipeline;

    let legacy = serde_json::json!({"id":"stable-id", "name":"Old display name", "steps":[]});
    let pipeline: Pipeline = serde_json::from_value(legacy).unwrap();
    assert_eq!(pipeline.id, "stable-id");
    let saved = serde_json::to_value(&pipeline).unwrap();
    assert_eq!(saved, serde_json::json!({"id":"stable-id", "steps":[]}));
    assert_eq!(serde_json::from_value::<Pipeline>(saved).unwrap(), pipeline);
    assert!(
        serde_json::from_value::<Pipeline>(serde_json::json!({
            "id":"stable-id", "steps":[], "unexpected":true
        }))
        .is_err()
    );
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
fn demo_factory_pipeline_requires_human_approval_before_commit() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/demo/.baton/pipelines/software-factory.yml");
    let source = std::fs::read_to_string(path).unwrap();
    let pipeline: baton_core::domain::project::Pipeline = serde_yaml_ng::from_str(&source).unwrap();
    let commit = pipeline
        .steps
        .iter()
        .find(|step| step.id == "commit")
        .unwrap();
    assert_eq!(commit.agent_id(), "factory-committer");
    assert!(commit.transitions.iter().any(|transition| {
        transition.outcome == baton_core::domain::project::Outcome::Completed
            && transition.to.is_none()
    }));
    let approval = pipeline
        .steps
        .iter()
        .find(|step| step.id == "approval")
        .unwrap();
    assert!(approval.is_human());
    assert!(approval.transitions.iter().any(|transition| {
        transition.outcome == baton_core::domain::project::Outcome::Approved
            && transition.to.as_deref() == Some("commit")
    }));
    let review = pipeline
        .steps
        .iter()
        .find(|step| step.id == "review")
        .unwrap();
    assert!(review.transitions.iter().any(|transition| {
        transition.outcome == baton_core::domain::project::Outcome::Approved
            && transition.to.as_deref() == Some("approval")
    }));
}

#[test]
fn project_factory_checks_ci_before_finishing() {
    use baton_core::domain::project::Outcome;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.baton/pipelines/software-factory.yml");
    let pipeline: baton_core::domain::project::Pipeline =
        serde_yaml_ng::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let step = |id: &str| pipeline.steps.iter().find(|s| s.id == id).unwrap();
    let routes_to = |id: &str, outcome, to: Option<&str>| {
        step(id)
            .transitions
            .iter()
            .any(|t| t.outcome == outcome && t.to.as_deref() == to)
    };
    assert!(routes_to("commit", Outcome::Completed, Some("ci")));
    assert!(routes_to("ci", Outcome::Completed, None));
    assert!(routes_to("ci", Outcome::ReviseBuild, Some("build")));
    assert!(routes_to("ci", Outcome::Blocked, Some("ci-help")));
    assert!(step("ci-help").is_human());
    assert!(routes_to("ci-help", Outcome::Responded, Some("ci")));
    let prompt = match &step("ci").kind {
        baton_core::domain::project::PipelineStepKind::Agent { prompt, .. } => prompt,
        _ => panic!("CI step must be an agent"),
    };
    assert!(prompt.contains("Inspect the existing pull request once"));
    assert!(prompt.contains("do not poll, sleep, or wait"));
    assert!(prompt.contains("PENDING (not"));
    assert!(prompt.contains("passed)"));
    assert!(prompt.contains("changed head makes the result indeterminate and blocked"));
    assert!(prompt.contains("actionable required-check failure"));
    assert!(prompt.contains("required-check set"));
    assert!(prompt.contains("state is unknown and must be blocked"));
    assert!(prompt.contains("known but pending/has no result"));
    assert!(prompt.contains("Authentication/API errors"));
    assert!(prompt.contains("task runs always begin at plan"));

    let plan_prompt = match &step("plan").kind {
        baton_core::domain::project::PipelineStepKind::Agent { prompt, .. } => prompt,
        _ => panic!("plan step must be an agent"),
    };
    assert!(plan_prompt.contains("reinspection"));
    assert!(plan_prompt.contains("automatically append"));
    assert!(plan_prompt.contains("up to 8 KiB"));
    assert!(plan_prompt.contains("Reopen & rerun"));
    assert!(plan_prompt.contains("Do not block"));
    assert!(plan_prompt.contains("task status is done"));
    assert!(plan_prompt.contains("retained worktree"));
    assert!(plan_prompt.contains("branch"));
    assert!(plan_prompt.contains("still exist"));
    let build_prompt = match &step("build").kind {
        baton_core::domain::project::PipelineStepKind::Agent { prompt, .. } => prompt,
        _ => panic!("build step must be an agent"),
    };
    assert!(build_prompt.contains("incoming plan"));
    assert!(build_prompt.contains("branch"));
    assert!(build_prompt.contains("is clean"));
    assert!(build_prompt.contains("report that no"));
    assert!(build_prompt.contains("implementation changes"));
    assert!(build_prompt.contains("same task worktree"));
    assert!(matches!(
        &step("build").kind,
        baton_core::domain::project::PipelineStepKind::Agent {
            include_task: Some(true),
            ..
        }
    ));
    for (agent, help, resume) in [
        ("review", "review-help", "review"),
        ("commit", "commit-help", "commit"),
    ] {
        assert!(routes_to(agent, Outcome::Blocked, Some(help)));
        assert!(step(help).is_human());
        assert!(routes_to(help, Outcome::Responded, Some(resume)));
        assert!(routes_to(help, Outcome::Rejected, None));
    }
    let review_prompt = match &step("review").kind {
        baton_core::domain::project::PipelineStepKind::Agent { prompt, .. } => prompt,
        _ => panic!("review step must be an agent"),
    };
    assert!(review_prompt.contains("reviewed diff/HEAD SHA"));
    assert!(review_prompt.contains("existing task branch's committed diff"));
    let commit_prompt = match &step("commit").kind {
        baton_core::domain::project::PipelineStepKind::Agent { prompt, .. } => prompt,
        _ => panic!("commit step must be an agent"),
    };
    assert!(commit_prompt.contains("changes that have not yet been committed"));
    assert!(commit_prompt.contains("compare the staged diff"));
    assert!(commit_prompt.contains("clean"));
    assert!(commit_prompt.contains("without committing or pushing"));
    assert!(commit_prompt.contains("pass through repository, branch, PR URL"));
    assert!(commit_prompt.contains("PR target base SHA"));
    assert!(commit_prompt.contains("pre-commit task HEAD SHA"));
    assert!(commit_prompt.contains("expected remote"));
    assert!(commit_prompt.contains("task-branch SHA or ABSENT"));
    assert!(commit_prompt.contains("incremental reviewed paths/fingerprint"));
    assert!(commit_prompt.contains("must be the pre-commit task HEAD"));
    assert!(commit_prompt.contains("exactly one new local"));
    assert!(commit_prompt.contains("patch matches"));
    assert!(commit_prompt.contains("expected ABSENT"));
    assert!(commit_prompt.contains("normal non-force push"));
    assert!(commit_prompt.contains("preserve every"));
    assert!(commit_prompt.contains("reviewer fact verbatim"));
    assert!(commit_prompt.contains("the worktree is clean"));
    assert!(commit_prompt.contains("do not"));
    assert!(commit_prompt.contains("create another commit"));
    assert!(commit_prompt.contains("remote"));
    assert!(commit_prompt.contains("branch SHA"));
    assert!(review_prompt.contains("patch fingerprint"));
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
fn legacy_progress_is_readable_but_not_a_pipeline_transition() {
    use baton_core::domain::project::Outcome;
    assert_eq!(
        serde_json::from_str::<Outcome>("\"progress\"").unwrap(),
        Outcome::Progress
    );
    let historical: baton_core::domain::pipeline::PipelineRun = serde_json::from_value(serde_json::json!({
        "id":"old", "pipelineId":"draft", "task":"task", "startedAt":"then",
        "endedAt":null, "durationMs":null, "status":"running",
        "steps":[{"stepId":"build", "agentId":"writer", "status":"succeeded", "input":null, "output":"partial", "error":null, "outcome":"progress", "attempt":1}]
    })).unwrap();
    assert_eq!(historical.steps[0].outcome, Some(Outcome::Progress));
    let mut value = example();
    value["pipelines"][0]["steps"][0]["transitions"] =
        serde_json::json!([{"outcome":"progress", "to":"plan"}]);
    assert!(
        parse_config(&value.to_string())
            .unwrap_err()
            .contains("only supported in historical run data")
    );
}

#[test]
fn legacy_system_prompt_and_optional_step_prompt_are_compatible() {
    let config: ProjectConfig = serde_json::from_value(serde_json::json!({
        "schemaVersion": 7,
        "agents": [{"id":"writer", "systemPrompt":"Write clearly", "skillIds":[]}],
        "pipelines": [{"id":"draft", "steps":[{"id":"write", "name":"Write", "type":"agent", "agentId":"writer"}]}]
    })).unwrap();
    assert_eq!(config.agents[0].instructions, "Write clearly");
    let saved = serde_json::to_value(config).unwrap();
    assert_eq!(saved["agents"][0]["instructions"], "Write clearly");
    assert!(saved["agents"][0].get("systemPrompt").is_none());
    assert_eq!(saved["pipelines"][0]["steps"][0]["prompt"], "");
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
            "/agents/0/model",
            serde_json::json!(" /gpt"),
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
            "agent.instructions must not be empty",
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
            "Legacy runtime profiles are no longer supported",
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
