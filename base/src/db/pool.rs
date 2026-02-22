use crate::configuration::DatabaseSettings;
use crate::error::Error;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;
use tracing::{debug, info};

/// Create a PostgreSQL connection pool
pub async fn create_pool(settings: &DatabaseSettings) -> Result<PgPool, Error> {
    info!("Creating database connection pool");
    debug!("Connecting to: {}", settings.connection_string_safe());

    let pool = PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .min_connections(settings.min_connections)
        .acquire_timeout(Duration::from_secs(settings.connection_timeout))
        .idle_timeout(Duration::from_secs(settings.idle_timeout))
        .connect(&settings.connection_string())
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("Failed to create connection pool: {}", e),
        })?;

    info!("Database connection pool created successfully");
    Ok(pool)
}

/// Test database connection
pub async fn test_connection(pool: &PgPool) -> Result<(), Error> {
    sqlx::query("SELECT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("Connection test failed: {}", e),
        })?;

    info!("Database connection test successful");
    Ok(())
}

/// Get PostGIS version
pub async fn get_postgis_version(pool: &PgPool) -> Result<String, Error> {
    let row: (String,) = sqlx::query_as("SELECT PostGIS_version()")
        .fetch_one(pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("Failed to get PostGIS version: {}", e),
        })?;

    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires database to be running
    async fn test_create_pool() {
        let settings = DatabaseSettings::default();
        let result = create_pool(&settings).await;

        // This test is ignored by default as it requires a running database
        // Run with: cargo test --package base -- --ignored
        if let Ok(pool) = result {
            assert!(test_connection(&pool).await.is_ok());
        }
    }
}
