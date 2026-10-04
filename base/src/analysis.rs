pub mod error;
pub mod flood;
pub mod flood_analyzer;
#[cfg(feature = "gdal-support")]
pub mod gdal_dem_source;
pub mod tile_grid;
pub mod trait_def;
pub mod types;

// Re-export commonly used items
pub use error::AnalysisError;
pub use flood::{CellInput, EdgeDepths, FloodTile, GRID_SIZE};
pub use flood_analyzer::{FloodAnalyzer, TileInputSource};
#[cfg(feature = "gdal-support")]
pub use gdal_dem_source::GdalDemSource;
pub use tile_grid::{run_tile_grid, TileId, TileState};
pub use trait_def::HazardAnalyzer;
pub use types::{
    BlendedReading, BlendedSource, ComputeModel, DataConfidence, FieldReport, HazardType,
    ReportSeverity, ScenarioInput, ScenarioParameter, ScenarioResult, WaterLevelBucket,
    WaterLevelSample,
};
