use async_trait::async_trait;

use super::error::AnalysisError;
use super::types::{ComputeModel, HazardType, ScenarioInput, ScenarioResult};

/// Core trait every hazard simulation implements.
///
/// Mirrors `loader::DataLoader` deliberately — this is the same facade
/// pattern, one trait per cross-cutting concern (loading data in, running
/// a hazard analysis). A `FloodAnalyzer` and an `EarthquakeAnalyzer` are
/// two structs implementing this trait, not two APIs.
///
/// Implementations are NOT required to use a tile grid. `compute_model`
/// tells the runner which shape of work this hazard needs:
/// `ComputeModel::TileGrid` for anything that flows across the map
/// (flood, wildfire) and `ComputeModel::PointSource` for anything that
/// falls off radially from one location (earthquake, asteroid impact).
/// Forcing a point-source hazard through tile machinery it doesn't need
/// wastes compute the "any spec" goal can't afford.
#[async_trait]
pub trait HazardAnalyzer: Send + Sync {
    /// Which hazard this analyzer computes.
    fn hazard_type(&self) -> HazardType;

    /// How this hazard wants its work distributed. See `ComputeModel` for
    /// why this is a hazard-level property, not a per-call flag.
    fn compute_model(&self) -> ComputeModel;

    /// Validate a scenario's parameters before running — catches bad
    /// units, out-of-range values, or missing required keys early rather
    /// than partway through a possibly-long simulation.
    fn validate(&self, input: &ScenarioInput) -> Result<(), AnalysisError>;

    /// Run the analysis and return a result ready for the snapshot store.
    ///
    /// For `ComputeModel::TileGrid` analyzers, this method owns the tile
    /// decomposition and the boundary-exchange loop internally — callers
    /// never see individual tiles, only the assembled result. See
    /// `flood.rs` for the reference tile/boundary-sync implementation.
    async fn run(&self, input: &ScenarioInput) -> Result<ScenarioResult, AnalysisError>;

    /// Name of this analyzer, for logging/debugging.
    fn name(&self) -> &str;
}
