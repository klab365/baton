use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_core::domain::project::ProjectConfig;
use baton_server::{AppState, router};
use tower::ServiceExt;

fn fixture() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "baton-pipeline-routes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(directory.join(".baton")).unwrap();
    std::fs::write(
        directory.join(".baton/config.json"),
        include_str!("../../baton-core/tests/fixture_config.json"),
    )
    .unwrap();
    directory
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn pipeline_routes_return_http_contracts() {
    let directory = fixture();
    let app = router(AppState::new(directory.clone()));

    let response = app
        .clone()
        .oneshot(Request::get("/api/pipelines").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let listing = json(response).await;
    let revision = listing["revision"].as_str().unwrap();
    assert_eq!(listing["pipelines"][0]["id"], "draft");
    assert_eq!(listing["agents"][0]["id"], "writer");

    let config: ProjectConfig =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    let mut pipeline = config.pipelines[0].clone();
    pipeline.id = "new-chain".into();
    let mut invalid = pipeline.clone();
    invalid.name = " ".into();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/pipelines")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "revision": revision, "pipeline": invalid }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    pipeline.name = "New chain".into();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/pipelines")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "revision": revision, "pipeline": pipeline }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/pipelines/new-chain")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = json(response).await;
    let updated_revision = detail["revision"].as_str().unwrap();
    assert_eq!(detail["pipeline"]["name"], "New chain");

    let response = app
        .clone()
        .oneshot(
            Request::delete("/api/pipelines/new-chain")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "revision": revision }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = app
        .clone()
        .oneshot(
            Request::delete("/api/pipelines/new-chain")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "revision": updated_revision }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .oneshot(
            Request::get("/api/pipelines/new-chain")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json(response).await["error"], "Pipeline not found");
    std::fs::remove_dir_all(directory).unwrap();
}
