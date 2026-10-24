use baton_adapters::GitWorktreeManager;
use baton_core::worktrees::WorktreeManager;
use std::{fs, path::PathBuf, process::Command};

struct Project(PathBuf);
impl Project {
    fn new(git: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "baton worktree ' ; {}-{}-{}",
            std::process::id(),
            baton_core::orchestration::utc_now().replace(':', "-"),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        if git {
            for args in [
                vec!["init"],
                vec![
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .arg("-C")
                        .arg(&path)
                        .args(args)
                        .output()
                        .unwrap()
                        .status
                        .success()
                );
            }
        }
        Self(path)
    }
    fn manager(&self) -> GitWorktreeManager {
        GitWorktreeManager {
            repository: self.0.clone(),
            base_revision: "HEAD".into(),
        }
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn allocation_resolves_base_and_refuses_reuse() {
    let project = Project::new(true);
    let manager = project.manager();
    let workspace = manager.allocate("one").unwrap();
    assert!(workspace.path.is_absolute());
    assert_ne!(workspace.path, project.0);
    assert!(workspace.path.join(".git").is_file());
    assert_eq!(workspace.branch, "baton/run-one");
    assert_eq!(workspace.base_revision.len(), 40);
    assert!(manager.allocate("one").is_err());
    assert!(manager.allocate("../escape").is_err());
    // Existing branch is not reused or reset.
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project.0)
            .args(["branch", "baton/run-collision"])
            .status()
            .unwrap()
            .success()
    );
    assert!(manager.allocate("collision").is_err());
    let mut unowned = workspace.clone();
    unowned.owner = "collision".into();
    unowned.path = workspace.path.parent().unwrap().join("collision");
    unowned.branch = "baton/run-collision".into();
    // A failed reservation must never grant cleanup ownership.
    assert!(manager.cleanup("collision", &mut unowned).is_err());
}
#[test]
fn concurrency_allocates_distinct_paths_and_reserves_same_owner_once() {
    let project = Project::new(true);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| project.manager().allocate("a").unwrap());
        let b = scope.spawn(|| project.manager().allocate("b").unwrap());
        assert_ne!(a.join().unwrap().path, b.join().unwrap().path);
        let a = scope.spawn(|| project.manager().allocate("same"));
        let b = scope.spawn(|| project.manager().allocate("same"));
        assert_ne!(a.join().unwrap().is_ok(), b.join().unwrap().is_ok());
    });
}
#[test]
fn rejects_non_git_and_invalid_base() {
    let project = Project::new(false);
    assert!(
        project
            .manager()
            .allocate("a")
            .unwrap_err()
            .contains("Git project")
    );
    let project = Project::new(true);
    let manager = GitWorktreeManager {
        repository: project.0.clone(),
        base_revision: "--help".into(),
    };
    assert!(manager.allocate("a").is_err());
}
#[test]
fn cleanup_requires_owner_identity_and_preserves_dirty_work() {
    let project = Project::new(true);
    let manager = project.manager();
    let mut a = manager.allocate("a").unwrap();
    let b = manager.allocate("b").unwrap();
    assert!(manager.cleanup("b", &mut a).is_err());
    assert!(a.path.exists());
    let mut forged = a.clone();
    forged.path = b.path.clone();
    assert!(manager.cleanup("a", &mut forged).is_err());
    fs::write(a.path.join("inspection.txt"), "valuable failure state").unwrap();
    assert!(manager.cleanup("a", &mut a).is_err());
    assert!(a.path.join("inspection.txt").exists());
    fs::remove_file(a.path.join("inspection.txt")).unwrap();
    manager.cleanup("a", &mut a).unwrap();
    assert!(!a.path.exists());
    assert_eq!(a.lifecycle, "removed");
    assert!(b.path.exists());
    assert!(manager.cleanup("a", &mut a).is_err());
    let mut changed = b.clone();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&b.path)
            .args(["checkout", "--detach"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(manager.cleanup("b", &mut changed).is_err());
    assert!(b.path.exists());
}
