use crate::{
    domain::project::{ConfigState, Skill},
    ports::{
        project_config::ProjectConfigRepository, skill_downloader::SkillDownloader,
        skills::SkillRepository,
    },
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
#[derive(Clone)]
pub struct SkillDownloaderResource(pub Arc<dyn SkillDownloader>);
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
#[derive(MediCommand)]
#[medi_command(error_type = SkillError)]
pub struct ImportSkill {
    pub url: String,
    pub overwrite: bool,
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
pub fn parse_skill(source: &str, id: String) -> Result<Skill, String> {
    let Some(after_open) = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
    else {
        return Ok(Skill {
            id,
            description: String::new(),
            metadata: String::new(),
            instructions: source.into(),
        });
    };
    let Some((frontmatter, instructions)) = after_open
        .split_once("\n---\n")
        .or_else(|| after_open.split_once("\r\n---\r\n"))
    else {
        return Err("Invalid SKILL.md frontmatter: missing closing --- delimiter".into());
    };
    let lines: Vec<_> = frontmatter.lines().collect();
    let description = lines
        .iter()
        .position(|line| line.starts_with("description:"))
        .map(|index| {
            let value = lines[index]["description:".len()..].trim();
            if matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
                let block = lines[index + 1..]
                    .iter()
                    .take_while(|line| line.starts_with(' ') || line.is_empty())
                    .map(|line| line.trim_start())
                    .collect::<Vec<_>>();
                if value.starts_with('>') {
                    block.join(" ")
                } else {
                    block.join("\n")
                }
            } else {
                value.trim_matches(['\"', '\'']).into()
            }
        })
        .unwrap_or_default();
    Ok(Skill {
        id,
        description,
        metadata: frontmatter.into(),
        instructions: instructions.into(),
    })
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
async fn import_handler(
    repository: SkillRepositoryResource,
    downloader: SkillDownloaderResource,
    command: ImportSkill,
) -> Result<(), SkillError> {
    let downloaded = downloader
        .0
        .download_skill(&command.url)
        .await
        .map_err(SkillError::Storage)?;
    let url = command.url;
    let id = url
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_string();
    let skill = parse_skill(&downloaded.skill_md, id).map_err(SkillError::Validation)?;
    validate(&skill).map_err(SkillError::Validation)?;
    let exists = repository
        .0
        .list()
        .map_err(SkillError::Storage)?
        .iter()
        .any(|existing| existing.id == skill.id);
    if exists && !command.overwrite {
        return Err(SkillError::Conflict("Skill ID already exists".into()));
    }
    if exists {
        repository.0.replace_imported(&skill, &downloaded.files)
    } else {
        repository.0.save_imported(&skill, &downloaded.files)
    }
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
#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn parses_folded_description_and_keeps_all_frontmatter_metadata() {
        let source = "---\nname: ponytail\ndescription: >\n  Lazy senior dev mode: smallest change.\n  A reply a busy human understands.\nargument-hint: [lite|full|ultra]\nlicense: MIT\n---\nInstructions go here.\n";
        let skill = parse_skill(source, "ponytail".into()).unwrap();
        assert_eq!(
            skill.description,
            "Lazy senior dev mode: smallest change. A reply a busy human understands."
        );
        assert!(skill.metadata.contains("argument-hint: [lite|full|ultra]"));
        assert!(skill.metadata.contains("license: MIT"));
        assert_eq!(skill.instructions, "Instructions go here.\n");
    }
}

medi_module! { pub manifest skills_feature; resources { skill_repository: crate::features::skills::SkillRepositoryResource; skill_downloader: crate::features::skills::SkillDownloaderResource; } commands {
crate::features::skills::ListSkills => crate::features::skills::list_handler;
crate::features::skills::SaveSkill => crate::features::skills::save_handler;
crate::features::skills::ImportSkill => crate::features::skills::import_handler;
crate::features::skills::DeleteSkill => crate::features::skills::delete_handler;
} }
