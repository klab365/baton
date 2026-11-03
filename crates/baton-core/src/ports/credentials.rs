use crate::domain::credentials::{Credential, CredentialType};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAuthentication {
    SecretInput,
    DeviceCode,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialTypeDescriptor {
    pub id: CredentialType,
    pub label: String,
    pub provider: Option<String>,
    pub supports_test: bool,
    pub authentication: CredentialAuthentication,
    pub configuration_fields: Vec<CredentialConfigurationField>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialConfigurationField {
    pub id: String,
    pub label: String,
    pub required: bool,
    pub secret: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDescriptor {
    pub id: String,
    pub label: String,
}

/// Lists models available through a locally stored credential.
#[async_trait]
pub trait CredentialModelCatalog: Send + Sync {
    async fn list_models(
        &self,
        credential: &Credential,
        secret: Option<&str>,
    ) -> Result<Vec<ModelDescriptor>, String>;
}

/// Tests a selected local credential without exposing its secret to callers.
#[async_trait]
pub trait CredentialConnectionTester: Send + Sync {
    async fn test_connection(
        &self,
        credential: &Credential,
        secret: Option<&str>,
    ) -> Result<(), String>;
}

/// Describes credential integrations supported by the composed adapters.
pub trait CredentialTypeCatalog: Send + Sync {
    fn list(&self) -> Vec<CredentialTypeDescriptor>;
    fn get(&self, id: &CredentialType) -> Option<CredentialTypeDescriptor>;
}

/// A lease held for a run or a credential mutation.
pub trait CredentialLease: Send {}

/// Stores local credential metadata and secret material outside project configuration.
#[async_trait]
pub trait CredentialRepository: Send + Sync {
    fn acquire_run_leases(
        &self,
        _ids: &[String],
    ) -> Result<Option<Box<dyn CredentialLease>>, String> {
        Ok(None)
    }
    fn acquire_delete_lease(&self, _id: &str) -> Result<Option<Box<dyn CredentialLease>>, String> {
        Ok(None)
    }
    async fn list(&self) -> Result<Vec<Credential>, String>;
    async fn get(&self, id: &str) -> Result<Option<Credential>, String>;
    async fn save(&self, credential: &Credential, secret: Option<&str>) -> Result<(), String>;
    async fn secret(&self, id: &str) -> Result<Option<String>, String>;
    /// Synchronous secret lookup for runtimes invoked on blocking worker threads.
    fn secret_sync(&self, id: &str) -> Result<Option<String>, String>;
    fn get_sync(&self, id: &str) -> Result<Option<Credential>, String>;
    async fn delete(&self, id: &str) -> Result<(), String>;
}
