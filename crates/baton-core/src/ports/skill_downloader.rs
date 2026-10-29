use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct DownloadedSkillFile {
    pub path: String,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DownloadedSkill {
    pub skill_md: String,
    pub files: Vec<DownloadedSkillFile>,
}

#[async_trait]
pub trait SkillDownloader: Send + Sync {
    async fn download_skill(&self, url: &str) -> Result<DownloadedSkill, String>;
}
