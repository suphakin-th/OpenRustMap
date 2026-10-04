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

/// How deep standing water is, in the same coarse, body-relative buckets a
/// person on the ground actually judges depth by — not a number they'd
/// have to guess in centimetres. Mirrors the dry/ankle/knee/waist/chest
/// picker pattern seen in comparable crowd flood-reporting tools (e.g.
/// K.A.R.A.'s Bangkok app), which turns out to be the right UX precedent:
/// false precision ("37cm") from a layperson's glance is worse than an
/// honest coarse bucket. `approx_m` gives each bucket a representative
/// depth so it can still be plotted on the same scale as a simulated
/// `WaterLevelSample` in metres.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaterLevelBucket {
    Dry,
    Ankle,
    Knee,
    Waist,
    Chest,
}

impl WaterLevelBucket {
    /// Representative depth in metres for this bucket — approximate by
    /// construction (see the type's own doc comment); never treat this as
    /// a precise measurement on par with a gauge reading or DEM sample.
    pub fn approx_m(&self) -> f64 {
        match self {
            WaterLevelBucket::Dry => 0.0,
            WaterLevelBucket::Ankle => 0.10,
            WaterLevelBucket::Knee => 0.45,
            WaterLevelBucket::Waist => 0.90,
            WaterLevelBucket::Chest => 1.30,
        }
    }
}

impl std::fmt::Display for WaterLevelBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaterLevelBucket::Dry => write!(f, "dry"),
            WaterLevelBucket::Ankle => write!(f, "ankle"),
            WaterLevelBucket::Knee => write!(f, "knee"),
            WaterLevelBucket::Waist => write!(f, "waist"),
            WaterLevelBucket::Chest => write!(f, "chest"),
        }
    }
}

/// A person's own judgment of how passable/dangerous a location is right
/// now — separate from water depth, because depth alone misses current
/// speed, downed power lines, open manholes, etc. that only a person on
/// the ground would know to flag.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportSeverity {
    Safe,
    Caution,
    Dangerous,
}

impl std::fmt::Display for ReportSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportSeverity::Safe => write!(f, "safe"),
            ReportSeverity::Caution => write!(f, "caution"),
            ReportSeverity::Dangerous => write!(f, "dangerous"),
        }
    }
}

/// One person's real-time, on-the-ground report of flood conditions at a
/// location — this is the crowd/sensor half of the aggregation this
/// project is building toward (simulation output is the other half, in
/// `WaterLevelSample`). Deliberately a separate type and a separate
/// table from `WaterLevelSample`, not a variant of it or a shared schema
/// with nullable fields for whichever kind doesn't apply:
///
/// - A `WaterLevelSample` exists on a regular timestep grid, produced in
///   bulk by one simulation run, always has a precise depth in metres,
///   and never needs confirmation — it's deterministic given its inputs.
/// - A `FieldReport` exists only when a person submits one, at whatever
///   irregular moment and location that happens, carries a coarse
///   human-judged bucket instead of a precise number, and *does* need a
///   trust signal — `confirm_count`/`dispute_count` are how this project
///   answers the same question K.A.R.A.'s own "weighs" pipeline stage
///   answers: how much to trust one report before blending it with
///   simulated or sensor data for the same location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldReport {
    /// None until the store assigns one on insert.
    pub id: Option<i64>,
    pub lat: f64,
    pub lon: f64,
    pub water_level: WaterLevelBucket,
    pub severity: ReportSeverity,
    /// Free-text note, e.g. "downed power line near the 7-Eleven" — the
    /// kind of hazard detail no sensor or simulation could ever surface.
    pub note: Option<String>,
    pub photo_url: Option<String>,
    pub reported_at: time::OffsetDateTime,
    pub confirm_count: i64,
    pub dispute_count: i64,
}

impl FieldReport {
    /// A quick, deliberately simple trust signal: net agreement among
    /// people who've seen this report in person, floored at zero so a
    /// heavily-disputed report doesn't read as "very trusted" in reverse.
    /// This is intentionally not a sophisticated weighting model — it's
    /// the simplest thing that lets an aggregation query prefer
    /// well-corroborated reports over single, unconfirmed ones, which is
    /// the actual problem being solved at this stage. A real reputation/
    /// weighting system (submitter history, photo presence, recency
    /// decay, official-source boost) is a legitimate later refinement on
    /// top of this, not a redesign of it.
    pub fn trust_score(&self) -> i64 {
        (self.confirm_count - self.dispute_count).max(0)
    }
}

/// Which kind of data actually answered a `blended_water_level` query —
/// the caller's way of knowing whether an eyewitness or a model produced
/// the number it's showing, since those carry very different trust and
/// very different staleness behavior (a report gets more stale faster
/// than a simulated projection does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendedSource {
    /// A field report with at least one net confirmation.
    FieldReport,
    /// A field report with no confirmations yet (or more disputes than
    /// confirmations) — still real, still worth showing, but the caller
    /// should treat it with more caution than a confirmed one.
    UnconfirmedFieldReport,
    /// No report existed nearby; this is a `FloodAnalyzer`-computed value.
    Simulated,
}

/// The answer `SnapshotStore::blended_water_level` actually returns —
/// one number, tagged with where it came from and how current it is, so
/// a caller never has to separately query reports and simulation and
/// reconcile them itself.
#[derive(Debug, Clone)]
pub struct BlendedReading {
    pub water_level_m: f64,
    pub source: BlendedSource,
    pub confidence: DataConfidence,
    pub as_of: time::OffsetDateTime,
}

/// One point-in-time water-level reading at one location, produced by a
/// `TileGrid` hazard run (flood, wildfire-equivalent intensity, etc.).
///
/// This is the unit the "how did the water level change over time at this
/// spot" question is actually answered from — a `ScenarioResult`'s
/// `intensity_field` is the whole-map picture at one instant; a flood
/// analyzer that wants a per-point timeline writes one `WaterLevelSample`
/// per (location, timestep) it computes, in addition to (not instead of)
/// its final `ScenarioResult`. The two are stored in different tables
/// (`scenario_snapshots` vs. `water_level_timeseries`) because they answer
/// different questions and need different indexes to answer them fast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaterLevelSample {
    pub scenario_name: String,
    pub lat: f64,
    pub lon: f64,
    pub observed_at: time::OffsetDateTime,
    pub water_level_m: f64,
    pub confidence: DataConfidence,
}
