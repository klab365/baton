use crate::{domain::project::Skill, ports::skill_downloader::DownloadedSkillFile};

pub trait SkillRepository: Send + Sync {
    fn list(&self) -> Result<Vec<Skill>, String>;
    fn save(&self, skill: &Skill) -> Result<(), String>;
    fn save_imported(&self, skill: &Skill, files: &[DownloadedSkillFile]) -> Result<(), String>;
    fn replace_imported(&self, skill: &Skill, files: &[DownloadedSkillFile]) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}
