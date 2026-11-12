mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_core::ports::{runs::RunRepository, tasks::TaskRepository};
use baton_server::{AppState, router};
use std::sync::OnceLock;
use tower::ServiceExt;

static PROVIDER_STORE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn fixture() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "baton-task-routes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(directory.join(".baton")).unwrap();
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
            std::process::Command::new("git")
                .arg("-C")
                .arg(&directory)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    common::write_config(
        &directory,
        &serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap(),
    );
    directory
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}
fn configure_fake(directory: &std::path::Path) {
    let mut config = common::read_config(directory);
    for agent in config["agents"].as_array_mut().unwrap() {
        agent["provider"] = "fake".into();
        agent["model"] = serde_json::Value::Null;
    }
    let steps = config["pipelines"][0]["steps"].as_array_mut().unwrap();
    steps.retain(|step| step["id"] == "write" || step["id"] == "review");
    for step in steps {
        step["transitions"] = serde_json::json!([]);
    }
    common::write_config(directory, &config);
}
fn input(title: &str) -> Body {
    Body::from(serde_json::json!({ "title": title, "description": "Feature details\n\nAcceptance criteria: Tests pass", "pipelineId": "draft" }).to_string())
}

#[tokio::test]
async fn tasks_do_not_depend_on_obsolete_local_provider_settings() {
    let _guard = PROVIDER_STORE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let directory = fixture();
    let provider_dir = directory.join("private-providers");
    std::fs::create_dir_all(&provider_dir).unwrap();
    std::fs::write(
        provider_dir.join("settings.json"),
        r#"[{"id":"other","name":"Other","kind":"openai","defaultModel":"other","baseUrl":null,"models":["other"],"status":"available"}]"#,
    ).unwrap();
    unsafe {
        std::env::set_var("BATON_PROVIDER_DIR", &provider_dir);
    }
    let app = router(AppState::new(directory.clone()));
    let response = app
        .oneshot(
            Request::post("/api/tasks")
                .header("content-type", "application/json")
                .body(input("Implement feature"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(json(response).await.get("configSnapshot").is_none());
    unsafe {
        std::env::remove_var("BATON_PROVIDER_DIR");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn task_completes_after_human_approval_and_repairs_stale_status() {
    let _guard = PROVIDER_STORE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let directory = fixture();
    configure_fake(&directory);
    let mut config = common::read_config(&directory);
    config["pipelines"][0]["steps"][1]["type"] = "human".into();
    config["pipelines"][0]["steps"][1]
        .as_object_mut()
        .unwrap()
        .remove("agentId");
    common::write_config(&directory, &config);
    let app = router(AppState::new(directory.clone()));
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header("content-type", "application/json")
                .body(input("Approval test"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let task = json(response).await;
    let id = task["id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let run = json(response).await;
    assert_eq!(run["status"], "waitingHuman");
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "doing");
    let run_id = run["id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/runs/{run_id}/actions"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"outcome":"approved","response":"Human feedback"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["status"], "succeeded");
    let repository =
        baton_adapters::repositories::filesystem_task::FilesystemTaskRepository(directory.clone());
    let mut stored = repository.load(id).unwrap();
    assert_eq!(stored.status, baton_core::domain::tasks::TaskStatus::Done);
    stored.status = baton_core::domain::tasks::TaskStatus::Doing;
    repository.save(&stored).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "done");
    assert_eq!(
        repository.load(id).unwrap().status,
        baton_core::domain::tasks::TaskStatus::Done
    );
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second_run = json(response).await;
    assert_eq!(second_run["status"], "waitingHuman");
    let prompt = second_run["task"].as_str().unwrap();
    assert!(prompt.contains(run_id));
    assert!(prompt.contains("1. write:"));
    assert!(prompt.contains("2. review:\nHuman feedback"));
    assert!(prompt.find("1. write:").unwrap() < prompt.find("2. review:").unwrap());
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "doing");
    // Complete the second run before testing a start rejected before run creation.
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let runs = json(response).await;
    let waiting = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["status"] == "waitingHuman")
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/runs/{}/actions",
                waiting["id"].as_str().unwrap()
            ))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"outcome":"approved"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stored = repository.load(id).unwrap();
    stored.description = "x".repeat(16385);
    repository.save(&stored).unwrap();
    let response = app
        .oneshot(
            Request::post(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!response.status().is_success());
    assert_eq!(
        repository.load(id).unwrap().status,
        baton_core::domain::tasks::TaskStatus::Done
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn task_routes_return_http_contracts() {
    let _guard = PROVIDER_STORE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let directory = fixture();
    configure_fake(&directory);
    let provider_dir = directory.join("private-providers");
    std::fs::create_dir_all(&provider_dir).unwrap();
    unsafe {
        std::env::set_var("BATON_PROVIDER_DIR", &provider_dir);
    }
    let app = router(AppState::new(directory.clone()));
    let response = app
        .clone()
        .oneshot(Request::get("/api/tasks").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await, serde_json::json!([]));

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header("content-type", "application/json")
                .body(input(" "))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/tasks")
                .header("content-type", "application/json")
                .body(input("Implement feature"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let task = json(response).await;
    let id = task["id"].as_str().unwrap().to_owned();
    assert_eq!(task["status"], "draft");
    assert!(task.get("configSnapshot").is_none());

    let response = app
        .clone()
        .oneshot(
            Request::put(format!("/api/tasks/{id}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "title": "Implement feature", "description": "Feature details", "pipelineId": "draft", "status": "draft" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(Request::get("/api/tasks").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await[0]["id"], id);

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["id"], id);

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let run = json(response).await;
    assert_eq!(run["taskId"], id);
    assert_eq!(run["status"], "succeeded");
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "done");
    let run_id = run["id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(json(response).await["events"].as_array().unwrap().len() >= 2);
    let response = app
        .clone()
        .oneshot(
            Request::put(format!("/api/tasks/{id}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "title": "Updated feature", "description": "Feature details\n\nAcceptance criteria: Tests pass", "pipelineId": "draft", "status": "done" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = json(response).await;
    assert_eq!(updated["title"], "Updated feature");
    assert_eq!(updated["status"], "done");
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let rerun = json(response).await;
    assert_eq!(rerun["taskId"], id);
    assert_eq!(rerun["status"], "succeeded");
    assert_eq!(run["task"], "Feature details");
    let prompt = rerun["task"].as_str().unwrap();
    assert!(prompt.starts_with("Feature details\n\nAcceptance criteria: Tests pass"));
    assert!(prompt.contains(run_id));
    assert_eq!(rerun["worktree"]["path"], run["worktree"]["path"]);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let history = json(response).await;
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == run["id"])
    );
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == rerun["id"])
    );
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await["status"], "done");
    let completed_run =
        baton_adapters::repositories::filesystem_run::FilesystemRunRepository(directory.clone())
            .load(run_id)
            .unwrap();
    assert_eq!(
        completed_run.worktree.as_ref().unwrap().lifecycle,
        "retained"
    );
    assert!(completed_run.worktree.as_ref().unwrap().path.exists());
    let response = app
        .clone()
        .oneshot(
            Request::delete(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app
        .oneshot(
            Request::get(format!("/api/tasks/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    unsafe {
        std::env::remove_var("BATON_PROVIDER_DIR");
    }
    std::fs::remove_dir_all(directory).unwrap();
}
