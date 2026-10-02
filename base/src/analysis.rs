pub mod error;
pub mod tile_grid;
pub mod trait_def;
pub mod types;

// Re-export commonly used items
pub use error::AnalysisError;
pub use tile_grid::{run_tile_grid, TileId, TileState};
pub use trait_def::HazardAnalyzer;
pub use types::{
    ComputeModel, DataConfidence, HazardType, ScenarioInput, ScenarioParameter, ScenarioResult,
    WaterLevelSample,
};
