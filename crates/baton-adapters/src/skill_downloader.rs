use async_trait::async_trait;
use baton_core::ports::skill_downloader::{DownloadedSkill, DownloadedSkillFile, SkillDownloader};

pub struct GithubSkillDownloader {
    client: reqwest::Client,
}
impl Default for GithubSkillDownloader {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SkillDownloader for GithubSkillDownloader {
    async fn download_skill(&self, url: &str) -> Result<DownloadedSkill, String> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| "Enter a valid skill URL.".to_string())?;
        let parts: Vec<_> = parsed
            .path_segments()
            .ok_or("Enter a skill folder URL.")?
            .collect();
        if parsed.host_str() != Some("github.com") {
            return Err(format!(
                "No skill importer is registered for host {:?}.",
                parsed.host_str().unwrap_or_default()
            ));
        }
        if parts.len() < 5 || parts[2] != "tree" {
            return Err("Enter a GitHub repository folder URL containing a skill folder.".into());
        }
        let root = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            parts[0],
            parts[1],
            parts[4..].join("/"),
            parts[3]
        );
        let mut pending = vec![(root, String::new())];
        let mut files = Vec::new();
        while let Some((endpoint, prefix)) = pending.pop() {
            let response = self
                .client
                .get(endpoint)
                .header("User-Agent", "baton")
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err("Could not download the skill folder from GitHub.".into());
            }
            let entries: Vec<serde_json::Value> =
                response.json().await.map_err(|error| error.to_string())?;
            for entry in entries {
                let name = entry
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("GitHub entry has no name")?;
                let path = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}/{name}")
                };
                match entry.get("type").and_then(serde_json::Value::as_str) {
                    Some("dir") => pending.push((
                        entry
                            .get("url")
                            .and_then(serde_json::Value::as_str)
                            .ok_or("GitHub directory has no URL")?
                            .to_owned(),
                        path,
                    )),
                    Some("file") => {
                        let download_url = entry
                            .get("download_url")
                            .and_then(serde_json::Value::as_str)
                            .ok_or("GitHub file has no download URL")?;
                        let bytes = self
                            .client
                            .get(download_url)
                            .header("User-Agent", "baton")
                            .send()
                            .await
                            .map_err(|error| error.to_string())?
                            .error_for_status()
                            .map_err(|error| error.to_string())?
                            .bytes()
                            .await
                            .map_err(|error| error.to_string())?
                            .to_vec();
                        files.push(DownloadedSkillFile {
                            path,
                            contents: bytes,
                        });
                    }
                    _ => {}
                }
            }
        }
        let skill_md = files
            .iter()
            .find(|file| file.path == "SKILL.md")
            .ok_or("Could not find SKILL.md in the GitHub folder.")?;
        let skill_md =
            String::from_utf8(skill_md.contents.clone()).map_err(|_| "SKILL.md must be UTF-8.")?;
        Ok(DownloadedSkill { skill_md, files })
    }
}
