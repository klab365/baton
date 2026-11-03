use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_server::{AppState, VERSION, router};
use tower::ServiceExt;

#[tokio::test]
async fn version_endpoint_reports_workspace_version() {
    let app = router(AppState::new(
        std::env::temp_dir().join("baton-version-test-nonexistent"),
    ));
    let response = app
        .oneshot(Request::get("/api/version").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["version"], VERSION);
    assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
}
