pub mod pool;

// Re-export commonly used items
pub use pool::{create_pool, test_connection, get_postgis_version};
