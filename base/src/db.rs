pub mod pool;

// Re-export commonly used items
pub use pool::{create_pool, get_postgis_version, test_connection};
