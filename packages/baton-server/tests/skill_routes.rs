use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use baton_server::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn skills_migrate_to_files_and_crud_protects_assignments() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".baton")).unwrap();
    let mut config: serde_json::Value =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    config["skills"] = serde_json::json!([{"id":"style","instructions":"Be concise"}]);
    config["agents"][0]["skillIds"] = serde_json::json!(["style"]);
    std::fs::write(dir.path().join(".baton/config.json"), config.to_string()).unwrap();
    let app = router(AppState::new(dir.path().into()));
    let response = app
        .clone()
        .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["skills"][0]["id"], "style");
    assert!(dir.path().join(".agents/skills/style/SKILL.md").exists());
    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".baton/config.json")).unwrap(),
    )
    .unwrap();
    assert!(persisted.get("skills").is_none());
    let response = app
        .clone()
        .oneshot(
            Request::delete("/api/skills/style")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    for (method, id, expected) in [
        ("POST", "new", 201),
        ("POST", "new", 409),
        ("PUT", "new", 204),
        ("PUT", "missing", 404),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(if method == "POST" {
                        "/api/skills".into()
                    } else {
                        format!("/api/skills/{id}")
                    })
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"skill":{"id":id,"description":"A test skill","instructions":"Do work"}}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let response = app
        .oneshot(
            Request::delete("/api/skills/new")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    assert!(!dir.path().join(".agents/skills/new").exists());
}
