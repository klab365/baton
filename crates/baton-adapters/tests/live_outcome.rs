use baton_adapters::{credentials::FilesystemCredentialRepository, runtime::ConfiguredRuntime};
use baton_core::{
    domain::project::{Outcome, parse_config},
    ports::runtime::AgentRuntime,
};
use std::{path::PathBuf, sync::Arc};

#[test]
#[ignore = "requires BATON_LIVE_PROJECT with an authenticated provider; incurs model costs"]
fn provider_recovers_missing_outcome_in_same_session() {
    let project =
        PathBuf::from(std::env::var_os("BATON_LIVE_PROJECT").expect("set BATON_LIVE_PROJECT"));
    let config =
        parse_config(&std::fs::read_to_string(project.join(".baton/config.json")).unwrap())
            .unwrap();
    let mut agent = config.agents.first().unwrap().clone();
    agent.skill_ids.clear();
    agent.allowed_tools = Some(vec!["report_outcome".into()]);
    agent.system_prompt = "For this test, answer your first request with the word READY without calling any tools. On a follow-up request, follow its instructions.".into();
    let credentials = FilesystemCredentialRepository::for_project(&project).unwrap();
    let auth_root = credentials.root_path().to_path_buf();
    let mut runtime = ConfiguredRuntime::new(Arc::new(credentials), auth_root, vec![]);
    let dir = tempfile::tempdir().unwrap();
    let output = runtime
        .execute_pipeline_step(
            dir.path(),
            &agent,
            "Answer READY first, without using tools.",
            &[Outcome::Completed],
            &|| false,
        )
        .expect("provider call failed");
    let decision = runtime.take_decision();
    let trace = runtime.take_trace();
    assert!(
        trace
            .iter()
            .any(|entry| entry.content.contains("You did not call report_outcome")),
        "model called report_outcome immediately; recovery was not exercised: {output}"
    );
    assert_eq!(
        decision.expect("missing recovered outcome").outcome,
        Outcome::Completed
    );
}
