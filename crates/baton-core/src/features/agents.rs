use crate::{
    domain::project::{Agent, ConfigSnapshot, ConfigState},
    ports::project_config::ProjectConfigRepository,
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Debug)]
pub enum AgentError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}

pub async fn list(repository: &dyn ProjectConfigRepository) -> Result<ConfigSnapshot, AgentError> {
    match repository.load().await.map_err(AgentError::Storage)? {
        ConfigState::Ready(snapshot) => Ok(snapshot),
        ConfigState::Missing => Err(AgentError::Conflict("Initialize the project first".into())),
        ConfigState::Invalid(message) => Err(AgentError::Conflict(message)),
    }
}

pub enum Mutation {
    Create(Agent),
    Update(String, Agent),
    Delete(String),
}

#[derive(Clone)]
pub struct AgentRepositoryResource(pub Arc<dyn ProjectConfigRepository>);

#[derive(MediCommand)]
#[medi_command(return_type = ConfigSnapshot, error_type = AgentError)]
pub struct ListAgents;

#[derive(MediCommand)]
#[medi_command(error_type = AgentError)]
pub struct MutateAgent {
    pub revision: String,
    pub mutation: Mutation,
}

pub async fn mutate(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    mutation: Mutation,
    profile_available: impl Fn(&str) -> bool,
) -> Result<(), AgentError> {
    let mut snapshot = list(repository).await?;
    if snapshot.revision != revision {
        return Err(AgentError::Conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    match mutation {
        Mutation::Create(agent) => {
            if snapshot
                .config
                .agents
                .iter()
                .any(|item| item.id == agent.id)
            {
                return Err(AgentError::Conflict("Agent ID already exists".into()));
            }
            ensure_available(&agent, &profile_available)?;
            snapshot.config.agents.push(agent);
        }
        Mutation::Update(id, agent) => {
            if id != agent.id {
                return Err(AgentError::Validation("Agent ID cannot be changed".into()));
            }
            ensure_available(&agent, &profile_available)?;
            let existing = snapshot
                .config
                .agents
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or(AgentError::NotFound)?;
            *existing = agent;
        }
        Mutation::Delete(id) => {
            let index = snapshot
                .config
                .agents
                .iter()
                .position(|item| item.id == id)
                .ok_or(AgentError::NotFound)?;
            if let Some((pipeline, step)) = snapshot.config.pipelines.iter().find_map(|pipeline| {
                pipeline
                    .steps
                    .iter()
                    .find(|step| step.agent_id == id)
                    .map(|step| (pipeline, step))
            }) {
                return Err(AgentError::Conflict(format!(
                    "Agent {id:?} is used by pipeline {:?}, step {:?}; remove or change that step before deleting the agent",
                    pipeline.name, step.name
                )));
            }
            snapshot.config.agents.remove(index);
        }
    }
    snapshot.config.validate().map_err(AgentError::Validation)?;
    repository
        .update(revision, &snapshot.config)
        .await
        .map_err(AgentError::Conflict)
}

#[medi_handler]
async fn list_agents_handler(
    repository: AgentRepositoryResource,
    _command: ListAgents,
) -> Result<ConfigSnapshot, AgentError> {
    list(repository.0.as_ref()).await
}

#[medi_handler]
async fn mutate_agent_handler(
    repository: AgentRepositoryResource,
    command: MutateAgent,
) -> Result<(), AgentError> {
    let snapshot = list(repository.0.as_ref()).await?;
    mutate(
        repository.0.as_ref(),
        &command.revision,
        command.mutation,
        |id| crate::domain::runtime_profiles::resolve(&snapshot.config, id).is_ok(),
    )
    .await
}

medi_module! {
    pub manifest agents_feature;
    resources { agent_repository: crate::features::agents::AgentRepositoryResource; }
    commands {
        crate::features::agents::ListAgents => crate::features::agents::list_agents_handler;
        crate::features::agents::MutateAgent => crate::features::agents::mutate_agent_handler;
    }
}

fn ensure_available(
    agent: &Agent,
    profile_available: &impl Fn(&str) -> bool,
) -> Result<(), AgentError> {
    if profile_available(&agent.runtime_profile_id) {
        Ok(())
    } else {
        Err(AgentError::Validation(format!(
            "Runtime profile {:?} is not configured; add it in Settings → Runtime profiles",
            agent.runtime_profile_id
        )))
    }
}
