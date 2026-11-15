mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_core::domain::project::{Agent, ProjectConfig};
use baton_server::{AppState, router};
use tower::ServiceExt;

fn fixture() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "baton-agent-routes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(directory.join(".baton")).unwrap();
    common::write_config(
        &directory,
        &serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap(),
    );
    directory
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn pi_model_route_returns_only_provider_scoped_models() {
    use std::os::unix::fs::PermissionsExt;

    let directory = fixture();
    let pi = directory.join("fake-pi");
    std::fs::write(
        &pi,
        "#!/bin/sh\nprintf 'provider model context\\nanthropic claude-4 100K\\nanthropic custom/model-v2 32K\\nopenai gpt-4.1 1M\\n'; echo hidden-secret >&2\n",
    )
    .unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = router(AppState::new_with_pi_command(
        directory.clone(),
        pi.into_os_string(),
    ));

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/pi/models/anthropic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json(response).await,
        serde_json::json!([
            {"id":"claude-4", "label":"claude-4"},
            {"id":"custom/model-v2", "label":"custom/model-v2"}
        ])
    );
    let invalid_provider = app
        .oneshot(
            Request::get("/api/pi/models/provider%3Becho")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_provider.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!json(invalid_provider).await.to_string().contains("secret"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn agent_routes_accept_direct_runtime_configuration_and_enforce_pipeline_references() {
    let directory = fixture();
    let app = router(AppState::new(directory.clone()));

    let listing = app
        .clone()
        .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.status(), StatusCode::OK);
    let listing = json(listing).await;
    assert!(listing.get("runtimeProfiles").is_none());
    let revision = listing["revision"].as_str().unwrap();
    let config: ProjectConfig =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    let mut agent: Agent = config.agents[0].clone();
    agent.id = "new-agent".into();
    agent.model.as_mut().unwrap().provider = " ".into();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/agents")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"revision": revision, "agent": agent}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    agent.model.as_mut().unwrap().provider = "openai".into();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/agents")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"revision": revision, "agent": agent}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let listing = json(
        app.clone()
            .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    let revision = listing["revision"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::delete("/api/agents/writer")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"revision": revision}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        json(response).await["error"]
            .as_str()
            .unwrap()
            .contains("draft")
    );

    let response = app
        .oneshot(
            Request::delete("/api/agents/new-agent")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"revision": revision}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    std::fs::remove_dir_all(directory).unwrap();
}
