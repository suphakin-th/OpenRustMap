use serde::{Deserialize, Serialize};

/// Database configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database_name: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connection_timeout: u64,
    pub idle_timeout: u64,
}

impl DatabaseSettings {
    /// Generate PostgreSQL connection string
    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password, self.host, self.port, self.database_name
        )
    }

    /// Generate connection string without password (for logging)
    pub fn connection_string_safe(&self) -> String {
        format!(
            "postgres://{}:***@{}:{}/{}",
            self.username, self.host, self.port, self.database_name
        )
    }
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 5432,
            username: "postgres".to_string(),
            password: String::new(),
            database_name: "openrustmap".to_string(),
            max_connections: 10,
            min_connections: 2,
            connection_timeout: 30,
            idle_timeout: 600,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_string() {
        let settings = DatabaseSettings {
            host: "localhost".to_string(),
            port: 5432,
            username: "testuser".to_string(),
            password: "testpass".to_string(),
            database_name: "testdb".to_string(),
            ..Default::default()
        };

        assert_eq!(
            settings.connection_string(),
            "postgres://testuser:testpass@localhost:5432/testdb"
        );
    }

    #[test]
    fn test_connection_string_safe() {
        let settings = DatabaseSettings {
            host: "localhost".to_string(),
            port: 5432,
            username: "testuser".to_string(),
            password: "secret123".to_string(),
            database_name: "testdb".to_string(),
            ..Default::default()
        };

        let safe_string = settings.connection_string_safe();
        assert!(!safe_string.contains("secret123"));
        assert!(safe_string.contains("***"));
    }
}
