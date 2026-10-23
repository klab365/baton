use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_server::{AppState, router};
use tower::ServiceExt;

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
    std::fs::write(
        directory.join(".baton/config.json"),
        include_str!("../../../examples/demo/.baton/config.json"),
    )
    .unwrap();
    directory
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}
fn input(title: &str) -> Body {
    Body::from(serde_json::json!({ "title": title, "description": "Feature details", "acceptanceCriteria": "Tests pass", "pipelineId": "draft" }).to_string())
}

#[tokio::test]
async fn task_routes_return_http_contracts() {
    let directory = fixture();
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
    assert_eq!(task["configSnapshot"]["pipelines"][0]["id"], "draft");

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
            Request::put(format!("/api/tasks/{id}"))
                .header("content-type", "application/json")
                .body(input("Updated feature"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["title"], "Updated feature");

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
    std::fs::remove_dir_all(directory).unwrap();
}
