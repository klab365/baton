use baton_adapters::{
    credentials::FilesystemCredentialRepository,
    repositories::yaml_project_config::YamlProjectConfigRepository, runtime::ConfiguredRuntime,
};
use baton_core::{
    domain::project::{ConfigState, Outcome},
    ports::{project_config::ProjectConfigRepository, runtime::AgentRuntime},
};
use std::{path::PathBuf, sync::Arc};

#[test]
#[ignore = "requires BATON_LIVE_PROJECT with an authenticated provider; incurs model costs"]
fn missing_outcome_is_retried_in_the_same_session() {
    let project =
        PathBuf::from(std::env::var_os("BATON_LIVE_PROJECT").expect("set BATON_LIVE_PROJECT"));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let ConfigState::Ready(snapshot) = runtime
        .block_on(YamlProjectConfigRepository::new(project.clone()).load())
        .unwrap()
    else {
        panic!("project configuration is unavailable")
    };
    let config = snapshot.config;
    let mut agent = config.agents.first().unwrap().clone();
    agent.skill_ids.clear();
    agent.allowed_tools = Some(vec!["report_outcome".into()]);
    agent.instructions = "For this test, answer your first request with the word READY without calling any tools. On a follow-up request, follow its instructions.".into();
    let credentials = FilesystemCredentialRepository::for_project(&project).unwrap();
    let auth_root = credentials.root_path().to_path_buf();
    let mut runtime = ConfiguredRuntime::new(Arc::new(credentials), auth_root, vec![]);
    let dir = tempfile::tempdir().unwrap();
    let _output = runtime
        .execute_pipeline_step(
            dir.path(),
            &agent,
            "Answer READY first, without using tools.",
            &[Outcome::Completed],
            &|| false,
        )
        .expect("provider call failed");
    assert_eq!(runtime.take_decision().unwrap().outcome, Outcome::Completed);
}
