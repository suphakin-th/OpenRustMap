use async_trait::async_trait;
use sqlx::PgPool;
use std::path::Path;
use uuid::Uuid;

use super::error::LoadError;
use super::types::{DataSourceMetadata, ImportStats, LoadedData, ValidationReport};

/// Core trait that all data loaders must implement
#[async_trait]
pub trait DataLoader: Send + Sync {
    /// Detect if this loader can handle the given file
    fn can_handle(&self, file_path: &Path) -> bool;

    /// Load and parse the file
    async fn load(&self, file_path: &Path) -> Result<LoadedData, LoadError>;

    /// Validate the loaded data
    async fn validate(&self, _data: &LoadedData) -> Result<ValidationReport, LoadError> {
        // Default implementation - can be overridden
        Ok(ValidationReport::valid())
    }

    /// Get metadata about the data source without fully loading it
    async fn metadata(&self, file_path: &Path) -> Result<DataSourceMetadata, LoadError>;

    /// Import data to database
    async fn import_to_db(
        &self,
        pool: &PgPool,
        data: LoadedData,
        source_id: Uuid,
    ) -> Result<ImportStats, LoadError>;

    /// Get the name of this loader (for logging/debugging)
    fn name(&self) -> &str;
}
