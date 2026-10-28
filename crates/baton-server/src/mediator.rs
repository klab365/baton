use baton_core::{
    agents_feature,
    features::{
        agents::AgentRepositoryResource, pipelines::PipelineRepositoryResource,
        runs::RunFeatureResources, runtime_profiles::RuntimeProfileRepositoryResource,
        tasks::TaskFeatureResources,
    },
    pipelines_feature,
    ports::project_config::ProjectConfigRepository,
    project_config, runs_feature, runtime_profiles_feature, tasks_feature,
};
use medi_rs::mediator;
use std::sync::Arc;

mediator! {
    pub struct ServerMediator {
        event_queue_capacity: 1;
        event_workers: 1;
        modules: [project_config, agents_feature, pipelines_feature, runtime_profiles_feature, tasks_feature, runs_feature];
    }
}

pub fn new(
    repository: Arc<dyn ProjectConfigRepository>,
    task_resources: TaskFeatureResources,
    run_resources: RunFeatureResources,
) -> ServerMediator {
    ServerMediator::builder()
        .repository(repository.clone())
        .agent_repository(AgentRepositoryResource(repository.clone()))
        .pipeline_repository(PipelineRepositoryResource(repository.clone()))
        .runtime_profile_repository(RuntimeProfileRepositoryResource(repository))
        .task_resources(task_resources)
        .run_resources(run_resources)
        .build()
}
