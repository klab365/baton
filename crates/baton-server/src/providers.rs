use crate::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::providers::Provider;
use rig_core::providers::{
    chatgpt::{
        self,
        auth::{AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore, TokenStore},
    },
    openai::OpenAIConfig,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Default)]
pub struct Providers {
    lock: Arc<tokio::sync::Mutex<()>>,
    flows: Arc<Mutex<HashMap<String, Flow>>>,
}
struct Flow {
    prompt: Arc<Mutex<Option<serde_json::Value>>>,
    task: tokio::task::JoinHandle<Result<Vec<String>, ()>>,
}
fn error(message: &str) -> ApiError {
    ApiError {
        status: axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        message: message.into(),
    }
}
fn storage_error() -> ApiError {
    error("Local provider storage is unavailable; check home directory permissions")
}

fn set_authenticated_models(provider: &mut Provider, result: Result<Vec<String>, ()>) {
    match result {
        Ok(models) if !models.is_empty() => {
            provider.status = "available".into();
            provider.models = models;
        }
        Ok(_) => {
            provider.status = "No ChatGPT models are available for this session".into();
            provider.models.clear();
        }
        Err(()) => {
            provider.status = "Could not load ChatGPT models; authenticate again".into();
            provider.models.clear();
        }
    }
}
fn root() -> Result<PathBuf, ApiError> {
    if !cfg!(unix) {
        return Err(error(
            "Permission-restricted provider storage currently requires Unix",
        ));
    }
    if let Some(directory) = std::env::var_os("BATON_PROVIDER_DIR") {
        return Ok(PathBuf::from(directory));
    }
    let home = std::env::var_os("HOME").ok_or_else(storage_error)?;
    Ok(PathBuf::from(home).join(".local/share/baton/providers"))
}
async fn read() -> Result<Vec<Provider>, ApiError> {
    let text = FileTokenStore::new(root()?)
        .load("settings")
        .await
        .map_err(|_| storage_error())?;
    text.map(|s| serde_json::from_str(&s).map_err(|_| storage_error()))
        .unwrap_or(Ok(vec![]))
}
async fn write(items: &[Provider]) -> Result<(), ApiError> {
    FileTokenStore::new(root()?)
        .save(
            "settings",
            &serde_json::to_string(items).map_err(|_| storage_error())?,
        )
        .await
        .map_err(|_| storage_error())
}
fn token_dir(id: &str) -> Result<PathBuf, ApiError> {
    Ok(root()?.join(id))
}
fn valid_id(id: &str) -> Result<(), ApiError> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| error("Provider not found"))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Save {
    name: String,
    kind: String,
    default_model: String,
    base_url: Option<String>,
    api_key: Option<String>,
}
async fn list(State(state): State<AppState>) -> Result<Json<Vec<Provider>>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    let mut items = read().await?;
    for p in &mut items {
        let finished = state
            .providers
            .flows
            .lock()
            .unwrap()
            .get(&p.id)
            .is_some_and(|f| f.task.is_finished());
        if finished {
            let flow = state.providers.flows.lock().unwrap().remove(&p.id).unwrap();
            set_authenticated_models(p, flow.task.await.unwrap_or(Err(())));
        }
    }
    write(&items).await?;
    Ok(Json(items))
}
async fn create(
    State(state): State<AppState>,
    Json(body): Json<Save>,
) -> Result<Json<Provider>, ApiError> {
    save(state, uuid::Uuid::new_v4().to_string(), body, false).await
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<Json<Provider>, ApiError> {
    valid_id(&id)?;
    save(state, id, body, true).await
}
async fn save(
    state: AppState,
    id: String,
    body: Save,
    existing: bool,
) -> Result<Json<Provider>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    let mut items = read().await?;
    if existing && !items.iter().any(|p| p.id == id) {
        return Err(error("Provider not found"));
    }
    let mut p = Provider {
        id: id.clone(),
        name: body.name,
        kind: body.kind,
        default_model: body.default_model,
        base_url: body.base_url.filter(|v| !v.is_empty()),
        models: vec![],
        status: "disconnected".into(),
    };
    p.validate().map_err(error)?;
    if let Some(base) = &p.base_url {
        let url =
            reqwest::Url::parse(base).map_err(|_| error("Enter a valid HTTPS API base URL"))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error(
                "API base URL must use HTTPS without credentials, query, or fragment",
            ));
        }
    }
    if let Some(old) = items.iter().find(|p| p.id == id)
        && old.kind != p.kind
    {
        return Err(error("Create a new provider to change provider type"));
    }
    if let Some(flow) = state.providers.flows.lock().unwrap().remove(&id) {
        flow.task.abort();
    }
    if let Some(key) = body.api_key.filter(|s| !s.is_empty()) {
        if p.kind != "openai" {
            return Err(error(
                "API keys are only supported for OpenAI API providers",
            ));
        }
        FileTokenStore::new(token_dir(&id)?)
            .save("api-key", &key)
            .await
            .map_err(|_| storage_error())?;
        p.status = "untested".into();
    }
    items.retain(|p| p.id != id);
    items.push(p.clone());
    write(&items).await?;
    Ok(Json(p))
}
async fn disconnect_inner(state: &AppState, id: &str, delete: bool) -> Result<(), ApiError> {
    valid_id(id)?;
    let flow = { state.providers.flows.lock().unwrap().remove(id) };
    if let Some(flow) = flow {
        flow.task.abort();
        let _ = flow.task.await;
    }
    let mut items = read().await?;
    let p = items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| error("Provider not found"))?;
    p.status = "disconnected".into();
    p.models.clear();
    let dir = token_dir(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|_| storage_error())?;
    }
    if delete {
        items.retain(|p| p.id != id);
    }
    write(&items).await
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<bool>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    disconnect_inner(&state, &id, true).await?;
    Ok(Json(true))
}
async fn disconnect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<bool>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    disconnect_inner(&state, &id, false).await?;
    Ok(Json(true))
}
async fn authenticate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<bool>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    valid_id(&id)?;
    let mut items = read().await?;
    let p = items
        .iter_mut()
        .find(|p| p.id == id && p.kind == "chatgpt")
        .ok_or_else(|| error("ChatGPT provider not found"))?;
    if state.providers.flows.lock().unwrap().contains_key(&id) {
        return Err(error(
            "Authentication is already in progress; disconnect to cancel",
        ));
    }
    p.status = "authenticating".into();
    p.models.clear();
    write(&items).await?;
    let prompt = Arc::new(Mutex::new(None));
    let output = prompt.clone();
    let store = FileTokenStore::new(token_dir(&id)?);
    let task = tokio::spawn(async move {
        let auth = Authenticator::new(
            AuthSource::OAuth,
            store,
            DeviceCodeHandler::new(move |p| {
                *output.lock().unwrap() = Some(
                    serde_json::json!({"verificationUri": p.verification_uri, "userCode": p.user_code}),
                );
            }),
            true,
        );
        let http = rig_reqwest::shared();
        let context = tokio::time::timeout(Duration::from_secs(900), auth.auth_context(&http))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let mut config = OpenAIConfig::with_key(&chatgpt::DIALECT, context.api_key);
        if let Some(account_id) = context.account_id {
            config = config.with_account_id(account_id);
        }
        let models = tokio::time::timeout(Duration::from_secs(30), config.client().list_models())
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        Ok(models.into_iter().map(|model| model.id).collect())
    });
    state
        .providers
        .flows
        .lock()
        .unwrap()
        .insert(id, Flow { prompt, task });
    Ok(Json(true))
}
async fn prompt(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    valid_id(&id)?;
    Ok(Json(
        state
            .providers
            .flows
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|f| f.prompt.lock().unwrap().clone())
            .unwrap_or(serde_json::Value::Null),
    ))
}
pub(crate) async fn available() -> Result<Vec<Provider>, ApiError> {
    Ok(read()
        .await?
        .into_iter()
        .filter(|provider| provider.status == "available" && !provider.models.is_empty())
        .collect())
}

pub(crate) async fn ensure_pipeline_available(
    config: &baton_core::project::ProjectConfig,
    pipeline_id: &str,
) -> Result<(), ApiError> {
    let providers = read().await?;
    // Keep existing portable projects usable until a local provider has been configured.
    if providers.is_empty() {
        return Ok(());
    }
    let pipeline = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == pipeline_id)
        .ok_or_else(|| error("Selected pipeline does not exist"))?;
    for step in &pipeline.steps {
        let agent = config
            .agents
            .iter()
            .find(|agent| agent.id == step.agent_id)
            .ok_or_else(|| error("Selected pipeline references an unknown agent"))?;
        if !providers.iter().any(|provider| {
            provider.id == agent.model.provider && provider.available(&agent.model.model)
        }) {
            return Err(error(&format!(
                "Agent {} requires local provider {} with available model {}; configure and test it in Settings",
                agent.id, agent.model.provider, agent.model.model
            )));
        }
    }
    Ok(())
}
async fn test(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Provider>, ApiError> {
    let _guard = state.providers.lock.lock().await;
    valid_id(&id)?;
    let mut items = read().await?;
    let p = items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| error("Provider not found"))?;
    if p.kind == "chatgpt" {
        return Err(error("Use Authenticate to verify the ChatGPT session"));
    }
    p.models.clear();
    p.status = "Connection failed; check API key, base URL, and model".into();
    let key = FileTokenStore::new(token_dir(&id)?)
        .load("api-key")
        .await
        .map_err(|_| storage_error())?;
    if let Some(key) = key {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| error("Connection test unavailable"))?;
        let response = client
            .get(format!(
                "{}/models",
                p.base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1")
                    .trim_end_matches('/')
            ))
            .bearer_auth(key)
            .send()
            .await;
        if let Ok(response) = response
            && response.status().is_success()
            && let Ok(value) = response.json::<serde_json::Value>().await
        {
            p.models = value["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            p.status = if p.models.contains(&p.default_model) {
                "available"
            } else {
                "Default model unavailable; edit model and test again"
            }
            .into();
        }
    }
    let result = p.clone();
    write(&items).await?;
    Ok(Json(result))
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/providers", get(list).post(create))
        .route(
            "/api/providers/{id}",
            axum::routing::put(update).delete(delete),
        )
        .route("/api/providers/{id}/test", axum::routing::post(test))
        .route(
            "/api/providers/{id}/disconnect",
            axum::routing::post(disconnect),
        )
        .route(
            "/api/providers/{id}/authenticate",
            axum::routing::post(authenticate),
        )
        .route("/api/providers/{id}/prompt", get(prompt))
        .layer(axum::middleware::map_response(
            |mut response: axum::response::Response| async move {
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_chatgpt_auth_persists_every_returned_model() {
        let mut provider = Provider {
            id: "local".into(),
            name: "ChatGPT".into(),
            kind: "chatgpt".into(),
            default_model: "model-one".into(),
            base_url: None,
            models: vec![],
            status: "authenticating".into(),
        };

        set_authenticated_models(
            &mut provider,
            Ok(vec!["model-one".into(), "model-two".into()]),
        );

        assert_eq!(provider.status, "available");
        assert_eq!(provider.models, ["model-one", "model-two"]);
        assert!(provider.available("model-two"));
    }

    #[test]
    fn empty_or_failed_chatgpt_model_listing_is_not_available() {
        let mut provider = Provider {
            id: "local".into(),
            name: "ChatGPT".into(),
            kind: "chatgpt".into(),
            default_model: "model-one".into(),
            base_url: None,
            models: vec![],
            status: "authenticating".into(),
        };

        set_authenticated_models(&mut provider, Ok(vec![]));
        assert_ne!(provider.status, "available");
        set_authenticated_models(&mut provider, Err(()));
        assert_ne!(provider.status, "available");
        assert!(provider.models.is_empty());
    }
}
