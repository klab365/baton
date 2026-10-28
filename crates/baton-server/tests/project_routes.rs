use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_server::{AppState, router};
use tower::ServiceExt;

fn fixture() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "baton-project-routes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn project_routes_dispatch_commands_and_return_api_dtos() {
    let directory = fixture();
    let app = router(AppState::new(directory.clone()));

    let response = app
        .clone()
        .oneshot(Request::get("/api/project").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["status"], "missing");

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/project/initialize")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(Request::get("/api/project").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let project = json(response).await;
    assert_eq!(project["status"], "ready");
    assert_eq!(project["config"]["schemaVersion"], 6);

    std::fs::remove_dir_all(directory).unwrap();
}
