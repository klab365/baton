use crate::{
    domain::project::{ConfigState, Skill},
    ports::{project_config::ProjectConfigRepository, skills::SkillRepository},
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Debug)]
pub enum SkillError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}
#[derive(Clone)]
pub struct SkillRepositoryResource(
    pub Arc<dyn SkillRepository>,
    pub Arc<dyn ProjectConfigRepository>,
);
#[derive(MediCommand)]
#[medi_command(return_type = Vec<Skill>, error_type = SkillError)]
pub struct ListSkills;
#[derive(MediCommand)]
#[medi_command(error_type = SkillError)]
pub struct SaveSkill {
    pub skill: Skill,
    pub update: bool,
}
#[derive(MediCommand)]
#[medi_command(error_type = SkillError)]
pub struct DeleteSkill {
    pub id: String,
}

pub fn validate(skill: &Skill) -> Result<(), String> {
    if skill.id.is_empty()
        || !skill
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        || skill.id == "."
        || skill.id == ".."
    {
        return Err("Invalid skill ID".into());
    }
    if skill.instructions.trim().is_empty() {
        return Err("Skill instructions must not be empty".into());
    }
    Ok(())
}
fn instruction_body(contents: &str) -> Result<&str, String> {
    if !contents.starts_with("---") {
        return Ok(contents);
    }
    let Some(after_open) = contents
        .strip_prefix("---\n")
        .or_else(|| contents.strip_prefix("---\r\n"))
    else {
        return Ok(contents);
    };
    let Some((_, body)) = after_open
        .split_once("\n---\n")
        .or_else(|| after_open.split_once("\r\n---\r\n"))
    else {
        return Err("Invalid SKILL.md frontmatter: missing closing --- delimiter".into());
    };
    Ok(body)
}
pub fn resolve(
    agent: &crate::domain::project::Agent,
    skills: &[Skill],
) -> Result<crate::domain::project::Agent, String> {
    let mut resolved = agent.clone();
    for id in &agent.skill_ids {
        let skill = skills
            .iter()
            .find(|skill| &skill.id == id)
            .ok_or_else(|| format!("Assigned skill {id:?} is missing from .agents/skills"))?;
        resolved.system_prompt.push_str(&format!(
            "\n\nAssigned skill {}:\n{}",
            skill.id,
            instruction_body(&skill.instructions)?
        ));
    }
    resolved.skill_ids.clear();
    Ok(resolved)
}
#[medi_handler]
async fn list_handler(
    repository: SkillRepositoryResource,
    _command: ListSkills,
) -> Result<Vec<Skill>, SkillError> {
    repository.0.list().map_err(SkillError::Storage)
}
#[medi_handler]
async fn save_handler(
    repository: SkillRepositoryResource,
    command: SaveSkill,
) -> Result<(), SkillError> {
    validate(&command.skill).map_err(SkillError::Validation)?;
    let exists = repository
        .0
        .list()
        .map_err(SkillError::Storage)?
        .iter()
        .any(|skill| skill.id == command.skill.id);
    if command.update && !exists {
        return Err(SkillError::NotFound);
    }
    if !command.update && exists {
        return Err(SkillError::Conflict("Skill ID already exists".into()));
    }
    repository
        .0
        .save(&command.skill)
        .map_err(SkillError::Storage)
}
#[medi_handler]
async fn delete_handler(
    repository: SkillRepositoryResource,
    command: DeleteSkill,
) -> Result<(), SkillError> {
    let config = match repository.1.load().await.map_err(SkillError::Storage)? {
        ConfigState::Ready(snapshot) => snapshot.config,
        _ => {
            return Err(SkillError::Conflict(
                "Project configuration is unavailable".into(),
            ));
        }
    };
    if let Some(agent) = config
        .agents
        .iter()
        .find(|agent| agent.skill_ids.contains(&command.id))
    {
        return Err(SkillError::Conflict(format!(
            "Skill is assigned to agent {:?}; remove the assignment first",
            agent.id
        )));
    }
    if !repository
        .0
        .list()
        .map_err(SkillError::Storage)?
        .iter()
        .any(|skill| skill.id == command.id)
    {
        return Err(SkillError::NotFound);
    }
    repository
        .0
        .delete(&command.id)
        .map_err(SkillError::Storage)
}
medi_module! { pub manifest skills_feature; resources { skill_repository: crate::features::skills::SkillRepositoryResource; } commands {
crate::features::skills::ListSkills => crate::features::skills::list_handler;
crate::features::skills::SaveSkill => crate::features::skills::save_handler;
crate::features::skills::DeleteSkill => crate::features::skills::delete_handler;
} }
