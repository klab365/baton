use crate::api::dto::Skill;
use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::features::skills::{DeleteSkill, ListSkills, SaveSkill, SkillError};
use serde::Deserialize;

impl From<SkillError> for ApiError {
    fn from(error: SkillError) -> Self {
        let (status, message) = match error {
            SkillError::NotFound => (StatusCode::NOT_FOUND, "Skill not found".into()),
            SkillError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            SkillError::Conflict(message) => (StatusCode::CONFLICT, message),
            SkillError::Storage(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        Self { status, message }
    }
}
async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let skills = state.mediator.send(ListSkills).await?;
    Ok(Json(serde_json::json!({"skills":skills})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    skill: Skill,
}
async fn create(
    State(state): State<AppState>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(SaveSkill {
            skill: body.skill.into(),
            update: false,
        })
        .await?;
    Ok(StatusCode::CREATED)
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    if id != body.skill.id {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "Skill ID cannot be changed".into(),
        });
    }
    state
        .mediator
        .send(SaveSkill {
            skill: body.skill.into(),
            update: true,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.mediator.send(DeleteSkill { id }).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/skills", get(list).post(create))
        .route(
            "/api/skills/{id}",
            axum::routing::put(update).delete(delete),
        )
}
