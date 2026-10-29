use crate::domain::project::Skill;

pub trait SkillRepository: Send + Sync {
    fn list(&self) -> Result<Vec<Skill>, String>;
    fn save(&self, skill: &Skill) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}
