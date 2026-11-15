use crate::{domain::runtime::RuntimeModel, ports::pi_models::PiModelCatalog};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Debug)]
pub enum PiModelError {
    Validation(String),
    Unavailable(String),
}

#[derive(Clone)]
pub struct PiModelCatalogResource(pub Arc<dyn PiModelCatalog>);

#[derive(MediCommand)]
#[medi_command(return_type = Vec<RuntimeModel>, error_type = PiModelError)]
pub struct ListPiModels {
    pub provider: String,
}

#[medi_handler]
async fn list_models_handler(
    catalog: PiModelCatalogResource,
    command: ListPiModels,
) -> Result<Vec<RuntimeModel>, PiModelError> {
    if command.provider.is_empty()
        || !command
            .provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(PiModelError::Validation(
            "Pi provider ID must contain only letters, numbers, dots, hyphens or underscores"
                .into(),
        ));
    }
    catalog
        .0
        .list_models(&command.provider)
        .await
        .map_err(PiModelError::Unavailable)
}

medi_module! {
    pub manifest pi_models_feature;
    resources {
        pi_model_catalog: crate::features::pi_models::PiModelCatalogResource;
    }
    commands {
        crate::features::pi_models::ListPiModels => crate::features::pi_models::list_models_handler;
    }
}
