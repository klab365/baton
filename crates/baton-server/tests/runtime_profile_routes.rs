use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_server::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn profiles_list_and_inspect_are_configuration_only() {
    let directory = std::env::temp_dir().join(format!("baton-profiles-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(directory.join(".baton")).unwrap();
    let mut config = baton_core::domain::project::ProjectConfig::CURRENT;
    config
        .runtime_profiles
        .push(baton_core::domain::runtime_profiles::RuntimeProfile {
            id: "fake.default".into(),
            name: "Tests".into(),
            runtime_kind: baton_core::domain::runtime_profiles::RuntimeKind::Fake,
            model: None,
        });
    std::fs::write(
        directory.join(".baton/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let app = router(AppState::new(directory.clone()));
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/runtime-profiles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let profiles: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(profiles[0]["id"], "fake.default");
    assert_eq!(profiles[0]["runtimeKind"], "fake");
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/runtime-profiles/fake.default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let inspected: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(inspected["profile"]["id"], "fake.default");
    assert!(inspected.get("status").is_none());
    for path in [
        "/api/runtime-profiles/missing",
        "/api/runtime-profiles/fake.default/models",
        "/api/providers",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
