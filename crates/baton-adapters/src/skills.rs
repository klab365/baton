use baton_core::{
    domain::project::Skill,
    ports::{skill_downloader::DownloadedSkillFile, skills::SkillRepository},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct FilesystemSkillRepository {
    root: PathBuf,
}
fn parse_skill(source: &str) -> Result<(String, String, String), String> {
    if !source.starts_with("---\n") && !source.starts_with("---\r\n") {
        return Ok((String::new(), String::new(), source.to_string()));
    }
    let start = source.find('\n').unwrap() + 1;
    let end = source[start..]
        .find("\n---")
        .map(|index| start + index)
        .ok_or("Invalid SKILL.md frontmatter: missing closing delimiter")?;
    let body_start = end + "\n---\n".len();
    let lines = source[start..end].lines().collect::<Vec<_>>();
    let mut description = String::new();
    if let Some(index) = lines
        .iter()
        .position(|line| line.starts_with("description:"))
    {
        let value = lines[index]["description:".len()..].trim();
        if matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
            let lines = lines[index + 1..]
                .iter()
                .take_while(|line| line.starts_with(' ') || line.is_empty())
                .map(|line| line.trim_start())
                .collect::<Vec<_>>();
            description = if value.starts_with('>') {
                lines.join(" ")
            } else {
                lines.join("\n")
            };
        } else {
            description = value.trim_matches(['"', '\'']).to_string();
        }
    }
    Ok((
        description,
        source[start..end].to_string(),
        source[body_start..].to_string(),
    ))
}
fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Skill paths cannot be symlinks: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
impl FilesystemSkillRepository {
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            root: project_dir.join(".agents/skills"),
        }
    }
    fn file(&self, id: &str) -> Result<PathBuf, String> {
        baton_core::features::skills::validate(&Skill {
            id: id.into(),
            description: "validation".into(),
            metadata: String::new(),
            instructions: "validation".into(),
        })?;
        reject_symlink(self.root.parent().unwrap())?;
        reject_symlink(&self.root)?;
        let dir = self.root.join(id);
        reject_symlink(&dir)?;
        let file = dir.join("SKILL.md");
        reject_symlink(&file)?;
        Ok(file)
    }
}
impl SkillRepository for FilesystemSkillRepository {
    fn list(&self) -> Result<Vec<Skill>, String> {
        reject_symlink(self.root.parent().unwrap())?;
        reject_symlink(&self.root)?;
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut skills = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            reject_symlink(&entry.path())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| "Skill ID must be UTF-8")?;
            let file = self.file(&id)?;
            if !file.exists() {
                continue;
            }
            let instructions = fs::read_to_string(&file)
                .map_err(|e| format!("Could not read {}: {e}", file.display()))?;
            let (description, metadata, instructions) = parse_skill(&instructions)?;
            let skill = Skill {
                id,
                description,
                metadata,
                instructions,
            };
            baton_core::features::skills::validate(&skill)?;
            skills.push(skill);
        }
        skills.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(skills)
    }
    fn save(&self, skill: &Skill) -> Result<(), String> {
        baton_core::features::skills::validate(skill)?;
        let file = self.file(&skill.id)?;
        let parent = file.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let frontmatter = if skill.metadata.is_empty() {
            format!(
                "name: {}\ndescription: |\n{}",
                skill.id,
                skill
                    .description
                    .lines()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        } else {
            skill.metadata.clone()
        };
        let instructions = format!("---\n{frontmatter}\n---\n{}", skill.instructions);
        let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            output
                .write_all(instructions.as_bytes())
                .map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, file).map_err(|e| e.to_string())
        })();
        let _ = fs::remove_file(temporary);
        result
    }
    fn save_imported(&self, skill: &Skill, files: &[DownloadedSkillFile]) -> Result<(), String> {
        self.save(skill)?;
        let root = self.file(&skill.id)?.parent().unwrap().to_path_buf();
        for file in files {
            let relative = Path::new(&file.path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(format!("Invalid imported skill file path {:?}", file.path));
            }
            if file.path == "SKILL.md" {
                continue;
            }
            let target = root.join(relative);
            let parent = target.parent().ok_or("Imported skill file has no parent")?;
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            fs::write(target, &file.contents).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    fn replace_imported(&self, skill: &Skill, files: &[DownloadedSkillFile]) -> Result<(), String> {
        let directory = self.file(&skill.id)?.parent().unwrap().to_path_buf();
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        }
        self.save_imported(skill, files)
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        let file = self.file(id)?;
        fs::remove_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_id_description_block_and_instructions() {
        let temp = tempfile::tempdir().unwrap();
        let repo = FilesystemSkillRepository::new(temp.path().into());
        let mut skill = Skill {
            id: "style".into(),
            description: "Tone rules\nKeep it concise".into(),
            metadata: String::new(),
            instructions: "Be concise".into(),
        };
        repo.save(&skill).unwrap();
        let saved = repo.list().unwrap().remove(0);
        let stored = fs::read_to_string(temp.path().join(".agents/skills/style/SKILL.md")).unwrap();
        assert!(stored.contains("description: |\n  Tone rules\n  Keep it concise"));
        assert!(stored.contains("name: style\n"));
        assert_eq!(saved.id, skill.id);
        assert_eq!(saved.description, "Tone rules\nKeep it concise");
        assert_eq!(saved.instructions, "Be concise");
        skill = saved;
        skill.instructions = "Be concise".into();
        repo.save(&skill).unwrap();
        assert_eq!(repo.list().unwrap()[0].description, skill.description);
        assert_eq!(repo.list().unwrap()[0].instructions, skill.instructions);
        assert_eq!(
            baton_core::features::skills::resolve(
                &baton_core::domain::project::Agent {
                    id: "a".into(),
                    system_prompt: String::new(),
                    runtime_kind: Default::default(),
                    model: None,
                    credential_id: None,
                    skill_ids: vec!["style".into()]
                },
                &repo.list().unwrap()
            )
            .unwrap()
            .system_prompt,
            "\n\nAssigned skill style:\nBe concise"
        );
        repo.delete("style").unwrap();
        assert!(repo.list().unwrap().is_empty());
        skill.id = "../escape".into();
        assert!(repo.save(&skill).is_err());
    }
    #[test]
    fn saves_imported_companion_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = FilesystemSkillRepository::new(temp.path().into());
        let skill = Skill {
            id: "bundle".into(),
            description: "A bundle".into(),
            metadata: String::new(),
            instructions: "Use references/example.md".into(),
        };
        repo.save_imported(
            &skill,
            &[DownloadedSkillFile {
                path: "references/example.md".into(),
                contents: b"Example".to_vec(),
            }],
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(
                temp.path()
                    .join(".agents/skills/bundle/references/example.md")
            )
            .unwrap(),
            "Example"
        );
    }
    #[test]
    fn reads_folded_frontmatter_description() {
        let source = "---\nname: ponytail\ndescription: >\n  Lazy senior dev mode.\n  Keep changes small.\nlicense: MIT\n---\nInstructions";
        let (description, metadata, instructions) = parse_skill(source).unwrap();
        assert_eq!(description, "Lazy senior dev mode. Keep changes small.");
        assert!(metadata.contains("license: MIT"));
        assert_eq!(instructions, "Instructions");
    }
    #[test]
    fn ignores_skill_directories_without_a_skill_file() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".agents/skills/deleted")).unwrap();
        let repo = FilesystemSkillRepository::new(temp.path().into());
        assert!(repo.list().unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".agents")).unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".agents/skills")).unwrap();
        let repo = FilesystemSkillRepository::new(temp.path().into());
        assert!(repo.list().is_err());
        assert!(repo.delete("test").is_err());
    }
}
