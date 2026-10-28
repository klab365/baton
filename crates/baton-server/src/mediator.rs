use baton_core::{
    agents_feature, credentials_feature,
    features::{
        agents::AgentRepositoryResource, credentials::CredentialRepositoryResource,
        pipelines::PipelineRepositoryResource, runs::RunFeatureResources,
        tasks::TaskFeatureResources,
    },
    pipelines_feature,
    ports::project_config::ProjectConfigRepository,
    project_config, runs_feature, tasks_feature,
};
use medi_rs::mediator;
use std::sync::Arc;

mediator! {
    pub struct ServerMediator {
        event_queue_capacity: 1;
        event_workers: 1;
        modules: [project_config, agents_feature, pipelines_feature, credentials_feature, tasks_feature, runs_feature];
    }
}

pub fn new(
    repository: Arc<dyn ProjectConfigRepository>,
    credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
    credential_types: Arc<dyn baton_core::ports::credentials::CredentialTypeCatalog>,
    credential_models: Arc<dyn baton_core::ports::credentials::CredentialModelCatalog>,
    task_resources: TaskFeatureResources,
    run_resources: RunFeatureResources,
) -> ServerMediator {
    ServerMediator::builder()
        .repository(repository.clone())
        .agent_repository(AgentRepositoryResource(repository.clone()))
        .pipeline_repository(PipelineRepositoryResource(repository.clone()))
        .credential_repository(CredentialRepositoryResource(
            credentials,
            credential_types,
            repository,
            credential_models,
        ))
        .task_resources(task_resources)
        .run_resources(run_resources)
        .build()
}
