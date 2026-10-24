use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baton_server::{AppState, router};
use std::sync::OnceLock;
use tower::ServiceExt;

static PROVIDER_STORE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn fixture() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "baton-provider-routes-{}-{}",
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
async fn call(
    app: axum::Router,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn provider_crud_keeps_secrets_out_of_project_and_responses() {
    // Environment variables are process-wide; serialize the test that selects its private store.
    let guard = PROVIDER_STORE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let directory = fixture();
    let secrets = directory.join("private-providers");
    unsafe {
        std::env::set_var("BATON_PROVIDER_DIR", &secrets);
    }
    let app = router(AppState::new(directory.clone()));
    let key = "sk-this-must-never-be-returned";
    let response = call(
        app.clone(),
        "POST",
        "/api/providers",
        serde_json::json!({
            "name": "Personal OpenAI", "kind": "openai", "defaultModel": "missing-model",
            "baseUrl": "", "apiKey": key
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created = json(response).await;
    let id = created["id"].as_str().unwrap();
    let created_text = created.to_string();
    assert!(!created_text.contains(key));
    assert_eq!(created["status"], "untested");

    let response = call(
        app.clone(),
        "GET",
        "/api/providers",
        serde_json::Value::Null,
    )
    .await;
    let listed = json(response).await;
    assert_eq!(listed[0]["name"], "Personal OpenAI");
    assert!(!listed.to_string().contains(key));
    assert!(
        !std::fs::read_to_string(directory.join(".baton/config.json"))
            .unwrap()
            .contains(key)
    );
    assert!(
        std::fs::read_dir(secrets.join(id))
            .unwrap()
            .next()
            .is_some()
    );

    let response = call(app.clone(), "PUT", &format!("/api/providers/{id}"), serde_json::json!({
        "name": "Renamed", "kind": "openai", "defaultModel": "missing-model", "baseUrl": "", "apiKey": ""
    })).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["name"], "Renamed");

    std::fs::remove_file(secrets.join(id).join("api-key.json")).unwrap();
    let response = call(
        app.clone(),
        "POST",
        &format!("/api/providers/{id}/test"),
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tested = json(response).await;
    assert!(!tested.to_string().contains(key));
    assert_ne!(tested["status"], "available");
    assert!(tested["models"].as_array().unwrap().is_empty());

    let response = call(
        app,
        "DELETE",
        &format!("/api/providers/{id}"),
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!secrets.join(id).exists());
    unsafe {
        std::env::remove_var("BATON_PROVIDER_DIR");
    }
    std::fs::remove_dir_all(directory).unwrap();
    drop(guard);
}

#[tokio::test]
async fn provider_rejects_invalid_credentials_and_chatgpt_base_url() {
    let guard = PROVIDER_STORE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let directory = fixture();
    unsafe {
        std::env::set_var("BATON_PROVIDER_DIR", directory.join("private-providers"));
    }
    let app = router(AppState::new(directory.clone()));
    let response = call(app.clone(), "POST", "/api/providers", serde_json::json!({
        "name": "ChatGPT", "kind": "chatgpt", "defaultModel": "gpt-4o", "baseUrl": "https://example.test", "apiKey": "secret"
    })).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let failure = json(response).await;
    assert!(!failure.to_string().contains("secret"));
    let response = call(app, "POST", "/api/providers", serde_json::json!({
        "name": "OpenAI", "kind": "openai", "defaultModel": "gpt-4o", "baseUrl": "http://example.test", "apiKey": "secret"
    })).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!json(response).await.to_string().contains("secret"));
    unsafe {
        std::env::remove_var("BATON_PROVIDER_DIR");
    }
    std::fs::remove_dir_all(directory).unwrap();
    drop(guard);
}
