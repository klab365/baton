use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use baton_server::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn create_only_rejects_an_existing_shared_id() {
    let directory = tempfile::tempdir().unwrap();
    let app = router(AppState::new(directory.path().to_owned()));
    let body = serde_json::json!({"name":"Local", "credentialType":"openai_api_key", "secret":"first", "createOnly":true}).to_string();
    let request = || {
        Request::put("/api/credentials/shared-openai")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::get("/api/credentials/unknown/models")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.clone().oneshot(Request::post("/api/credentials/shared-openai/authenticate").header("content-type", "application/json").body(Body::from(serde_json::json!({"name":"shared-openai", "credentialType":"chatgpt_session", "createOnly":true}).to_string())).unwrap()).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.oneshot(
            Request::delete("/api/credentials/shared-openai")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::NO_CONTENT
    );
}
