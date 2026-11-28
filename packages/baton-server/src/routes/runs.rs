use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use baton_core::domain::pipeline::{PipelineRun, StepRun};
use baton_core::features::runs::{CancelRun, GetRun, ListRuns, RespondToRun};
use serde_json::Value;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runs", get(list))
        .route("/api/runs/{id}", get(detail))
        .route("/api/runs/{id}/events", get(updates))
        .route("/api/runs/{id}/cancel", post(cancel))
        .route("/api/runs/{id}/actions", post(human_action))
}

fn summary(steps: &[StepRun]) -> Value {
    baton_core::domain::pipeline::usage_summary(steps)
}

fn run_response(run: PipelineRun) -> Result<Value, ApiError> {
    let summary = summary(&run.steps);
    let mut value =
        serde_json::to_value(run).map_err(|error| ApiError::internal(error.to_string()))?;
    value["usageSummary"] = summary;
    Ok(value)
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let runs = state
        .mediator
        .send(ListRuns)
        .await
        .map_err(ApiError::internal)?;
    runs.into_iter()
        .map(run_response)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let run = state
        .mediator
        .send(GetRun { id })
        .await
        .map_err(|error| ApiError {
            status: if error.contains("No such file") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            message: error,
        })?;
    run_response(run).map(Json)
}

async fn updates(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    axum::response::sse::Sse<
        impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ApiError,
> {
    use tokio_stream::StreamExt;
    let receiver = state.run_updates.subscribe();
    let _ = detail(State(state), Path(id.clone())).await?;
    let changes =
        tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(
            move |event| match event {
                Ok(run_id) if run_id != id => None,
                _ => Some(Ok(axum::response::sse::Event::default()
                    .event("update")
                    .data("changed"))),
            },
        );
    let initial = tokio_stream::once(Ok(axum::response::sse::Event::default()
        .event("update")
        .data("connected")));
    Ok(axum::response::sse::Sse::new(initial.chain(changes))
        .keep_alive(axum::response::sse::KeepAlive::default()))
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PipelineRun>, ApiError> {
    state
        .mediator
        .send(CancelRun { id })
        .await
        .map(Json)
        .map_err(|message| {
            if message.starts_with("Only running or waiting") {
                ApiError::conflict(message)
            } else {
                ApiError::internal(message)
            }
        })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanAction {
    outcome: baton_core::domain::project::Outcome,
    #[serde(default)]
    response: String,
}

async fn human_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(action): Json<HumanAction>,
) -> Result<Json<PipelineRun>, ApiError> {
    state
        .mediator
        .send(RespondToRun {
            id,
            outcome: action.outcome,
            response: action.response,
        })
        .await
        .map(Json)
        .map_err(|message| {
            if message.contains("Run action is already in progress")
                || message.contains("not waiting for a human")
            {
                ApiError::conflict(message)
            } else {
                ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message,
                }
            }
        })
}

#[cfg(test)]
mod usage_summary_tests {
    use super::*;

    fn step(usage: Value) -> StepRun {
        serde_json::from_value(serde_json::json!({
            "stepId":"build", "agentId":"worker", "status":"succeeded",
            "input":null, "output":null, "error":null, "usage":usage
        }))
        .unwrap()
    }

    #[test]
    fn summary_only_totals_complete_comparable_usage() {
        let priced = |cost: &str, currency: &str| {
            serde_json::json!({
                "estimatedCost":cost, "currency":currency, "priceUnit":"perMillionTokens", "coverage":"complete"
            })
        };
        let total = summary(&[step(priced("0.1", "USD")), step(priced("0.02", "USD"))]);
        assert_eq!(total["cost"], "0.12");
        let mixed = summary(&[step(priced("0.1", "USD")), step(priced("0.02", "EUR"))]);
        assert_eq!(mixed["coverage"], "unavailable");
        let missing = summary(&[step(Value::Null)]);
        assert_eq!(missing["cost"], Value::Null);
    }

    #[test]
    fn run_response_includes_additive_usage_summary() {
        let run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":"p", "task":"t", "startedAt":"now",
            "status":"succeeded", "steps":[{
                "stepId":"build", "agentId":"worker", "status":"succeeded",
                "input":null, "output":null, "error":null,
                "usage":{"estimatedCost":"0.03", "currency":"USD", "priceUnit":"perMillionTokens", "coverage":"complete"}
            }]
        })).unwrap();
        let response = run_response(run).unwrap_or_else(|_| panic!("valid run serializes"));
        assert_eq!(response["usageSummary"]["cost"], "0.03");
        assert_eq!(response["steps"][0]["usage"]["currency"], "USD");
    }
}
