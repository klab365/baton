//! Local, non-authoritative notes shared by attempts in one run.
use std::{fs, io::Write, path::Path};

const MAX_MEMORY_BYTES: u64 = 256 * 1024;

fn directory(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_dir() {
                Ok(())
            } else {
                Err(format!(
                    "Run memory directory is not a directory: {}",
                    path.display()
                ))
            }
        }
        Err(error) => Err(format!("Create run memory directory: {error}")),
    }
}

pub(super) fn prepare(workspace: &Path, id: &str, task: &str) -> Result<String, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Invalid run memory ID".into());
    }
    let baton = workspace.join(".baton");
    directory(&baton)?;
    let work = baton.join("work");
    directory(&work)?;
    let ignore = work.join(".gitignore");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    match options.open(&ignore) {
        Ok(mut file) => file.write_all(b"*\n").map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&ignore).map_err(|error| error.to_string())?;
            if !metadata.file_type().is_file() || metadata.len() > MAX_MEMORY_BYTES {
                return Err("Unsafe run memory ignore file".into());
            }
            let contents = fs::read_to_string(&ignore).map_err(|error| error.to_string())?;
            if contents.trim() != "*" {
                return Err("Existing .baton/work/.gitignore does not ignore all run data".into());
            }
        }
        Err(error) => return Err(format!("Protect run memory from Git: {error}")),
    }
    let path = work.join(format!("{id}.md"));
    if !path.exists() && fs::symlink_metadata(&path).is_err() {
        let initial = format!(
            "# Task\n\n{task}\n\n# Run working notes\n\nNon-authoritative notes for this run. Do not record secrets.\n"
        );
        if initial.len() as u64 > MAX_MEMORY_BYTES {
            return Err("Initial run memory exceeds 256 KiB".into());
        }
        let temporary = work.join(format!(".memory-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            file.write_all(initial.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| error.to_string())?;
            match fs::hard_link(&temporary, &path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(format!("Create run memory: {error}")),
            }
        })();
        let _ = fs::remove_file(&temporary);
        result?;
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_MEMORY_BYTES {
        return Err("Run memory must be a regular file of at most 256 KiB".into());
    }
    Ok(format!(".baton/work/{id}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_per_run_and_existing_contents_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        prepare(temp.path(), "run-1", "task").unwrap();
        let first = temp.path().join(".baton/work/run-1.md");
        assert!(
            fs::read_to_string(&first)
                .unwrap()
                .contains("# Task\n\ntask")
        );
        fs::write(&first, "next action").unwrap();
        prepare(temp.path(), "run-1", "task").unwrap();
        prepare(temp.path(), "run-2", "task").unwrap();
        assert_eq!(fs::read_to_string(first).unwrap(), "next action");
        assert!(temp.path().join(".baton/work/run-2.md").is_file());
        assert!(prepare(temp.path(), "../other", "task").is_err());
    }

    #[test]
    fn refuses_oversized_notes_and_unsafe_ignore_file() {
        let temp = tempfile::tempdir().unwrap();
        prepare(temp.path(), "run", "task").unwrap();
        fs::write(
            temp.path().join(".baton/work/run.md"),
            vec![b'x'; MAX_MEMORY_BYTES as usize],
        )
        .unwrap();
        assert!(prepare(temp.path(), "run", "task").is_ok());
        fs::write(
            temp.path().join(".baton/work/run.md"),
            vec![b'x'; MAX_MEMORY_BYTES as usize + 1],
        )
        .unwrap();
        assert!(prepare(temp.path(), "run", "task").is_err());
        fs::write(temp.path().join(".baton/work/.gitignore"), "*\n!*.md\n").unwrap();
        assert!(prepare(temp.path(), "other", "task").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_memory() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        prepare(temp.path(), "first", "task").unwrap();
        symlink(
            temp.path().join("outside"),
            temp.path().join(".baton/work/next.md"),
        )
        .unwrap();
        assert!(prepare(temp.path(), "next", "task").is_err());
    }
}
