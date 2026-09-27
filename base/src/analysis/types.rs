use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::loader::BoundingBox;

/// Which hazard a scenario or result belongs to.
///
/// Each variant maps to one `HazardAnalyzer` implementation. Adding a new
/// hazard means adding a variant here plus a new analyzer — it does not
/// change the trait or the storage schema.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HazardType {
    Flood,
    Earthquake,
    Wildfire,
    AsteroidImpact,
}

impl std::fmt::Display for HazardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HazardType::Flood => write!(f, "flood"),
            HazardType::Earthquake => write!(f, "earthquake"),
            HazardType::Wildfire => write!(f, "wildfire"),
            HazardType::AsteroidImpact => write!(f, "asteroid_impact"),
        }
    }
}

/// How much confidence a caller should place in a piece of data.
///
/// Mirrors the observed/ vs modeled/ vs scenarios/ split used in the
/// on-disk dataset layout: real measurements, results of an actual
/// simulation run, and pure what-if hypotheticals are never allowed to
/// look the same in storage or in an API response.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataConfidence {
    /// Measured or surveyed — DEM pixels, gauge readings, OSM geometry.
    Observed,
    /// Output of a real simulation run against observed inputs.
    Modeled,
    /// A what-if input was substituted for an observed one (e.g. +1m surge).
    Hypothetical,
}

/// A single input parameter to a hazard scenario, with enough metadata to
/// tell a real physical reading apart from a knob the user turned.
///
/// This is what lets a caller "add and remove factors in detail" — soil
/// water retention, wind direction, impactor mass — without the trait
/// needing a new method per parameter. Each analyzer defines which keys
/// it reads and what units/ranges are valid; unrecognized keys are ignored
/// rather than rejected, so scenarios stay forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioParameter {
    pub key: String,
    pub value: f64,
    pub unit: String,
    pub confidence: DataConfidence,
}

/// Everything one call to `HazardAnalyzer::run` needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioInput {
    pub hazard: HazardType,
    pub bbox: BoundingBox,
    /// Zoom level for tile-based hazards (flood, wildfire). Ignored by
    /// point-source hazards (earthquake, asteroid impact) — see
    /// `HazardAnalyzer::compute_model`.
    pub zoom: Option<u8>,
    pub parameters: Vec<ScenarioParameter>,
    /// Free-form label for the run, e.g. "bangkok-sep2026-observed-rainfall".
    /// This is the primary key a caller uses to find the run again later
    /// in the snapshot store.
    pub scenario_name: String,
}

impl ScenarioInput {
    pub fn parameter(&self, key: &str) -> Option<f64> {
        self.parameters
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.value)
    }

    pub fn parameter_map(&self) -> HashMap<String, f64> {
        self.parameters
            .iter()
            .map(|p| (p.key.clone(), p.value))
            .collect()
    }
}

/// How an analyzer wants its work distributed. This is a declaration, not
/// a request — the runner decides how many tiles/workers actually run
/// concurrently based on the machine it's on (see `db::sqlite_snapshot`
/// for where run history is kept so a low-spec machine can resume instead
/// of recomputing from zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeModel {
    /// Grid decomposed into Web-Mercator tiles; adjacent tiles exchange
    /// boundary state each timestep (shallow water flow, fire spread).
    /// Never split across machines via network sockets — the boundary
    /// exchange needs to happen every timestep, and network latency would
    /// dominate the actual compute. Split across threads/tasks on one
    /// machine instead (see flood.rs for the reference implementation).
    TileGrid,
    /// Single point-source calculation with closed-form distance falloff
    /// (seismic attenuation, blast/thermal radius). No tiling, no
    /// boundary sync — just evaluate the falloff function at whatever
    /// points the caller asks about.
    PointSource,
}

/// Output of one hazard analysis run, independent of which hazard produced it.
///
/// `affected_area` and `intensity_field` are intentionally generic —
/// flood depth, peak ground acceleration, and fire spread probability all
/// end up here as "how strong was this hazard at this point," which is
/// what every downstream consumer (map render, snapshot store, report)
/// actually needs, regardless of the physics that produced the number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub hazard: HazardType,
    pub scenario_name: String,
    pub confidence: DataConfidence,
    pub bbox: BoundingBox,
    /// GeoJSON FeatureCollection — polygons/points carrying an `intensity`
    /// property in whatever unit the hazard uses (metres for flood depth,
    /// g for peak ground acceleration, etc.). Kept as a JSON value rather
    /// than a typed geo_types collection so it serializes straight into
    /// the SQLite snapshot store without a conversion step.
    pub intensity_field: serde_json::Value,
    pub parameters_used: Vec<ScenarioParameter>,
    pub computed_at: time::OffsetDateTime,
    pub duration_seconds: f64,
}
