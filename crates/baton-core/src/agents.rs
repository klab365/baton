use crate::{
    project::{Agent, ConfigSnapshot, ConfigState},
    project_config::ProjectConfigRepository,
};

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

pub async fn mutate(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    mutation: Mutation,
    model_available: impl Fn(&str, &str) -> bool,
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
            ensure_available(&agent, &model_available)?;
            snapshot.config.agents.push(agent);
        }
        Mutation::Update(id, agent) => {
            if id != agent.id {
                return Err(AgentError::Validation("Agent ID cannot be changed".into()));
            }
            ensure_available(&agent, &model_available)?;
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

fn ensure_available(
    agent: &Agent,
    model_available: &impl Fn(&str, &str) -> bool,
) -> Result<(), AgentError> {
    if model_available(&agent.model.provider, &agent.model.model) {
        Ok(())
    } else {
        Err(AgentError::Validation(format!(
            "Provider {:?} with model {:?} is not configured and verified locally; configure and test it in Settings",
            agent.model.provider, agent.model.model
        )))
    }
}
