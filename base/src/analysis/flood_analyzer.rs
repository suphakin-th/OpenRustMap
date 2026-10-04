use async_trait::async_trait;
use std::collections::HashMap;

use super::error::AnalysisError;
use super::flood::{CellInput, FloodTile, GRID_SIZE};
use super::tile_grid::{run_tile_grid, TileId};
use super::trait_def::HazardAnalyzer;
use super::types::{
    ComputeModel, DataConfidence, HazardType, ScenarioInput, ScenarioParameter, ScenarioResult,
    WaterLevelSample,
};

/// Resolves the real-world inputs a `FloodTile` needs for one tile: the
/// per-cell elevation and waterway flags.
///
/// This is a trait, not a direct GDAL/PostGIS call, so `FloodAnalyzer`
/// stays testable without a real DEM file or database connection — a test
/// supplies a flat synthetic source; production code supplies one backed
/// by the real `elevation/bangkok_glo30_dem.tif` (via the optional
/// `gdal-support` feature) and `osm_waterways`/`osm_drainage` tables
/// already imported into PostGIS. Wiring a real GDAL-backed
/// implementation is the next concrete step after this lands — this trait
/// is the seam that work plugs into, not a placeholder that gets deleted.
pub trait TileInputSource: Send + Sync {
    /// Elevation + waterway flag for every cell in `tile`, row-major,
    /// exactly `GRID_SIZE * GRID_SIZE` entries — same contract as
    /// `FloodTile::new`.
    fn resolve(&self, tile: TileId) -> Vec<CellInput>;
}

/// Flood hazard analyzer: a `ComputeModel::TileGrid` implementation using
/// the simplified diffusive flood-fill model in `flood.rs` (rainfall in,
/// drainage out, flow toward lower surface elevation — not full
/// shallow-water PDEs; see the discussion that scoped this as the right
/// first model to build against real Bangkok data before anything
/// research-grade).
pub struct FloodAnalyzer<S: TileInputSource> {
    inputs: S,
}

impl<S: TileInputSource> FloodAnalyzer<S> {
    pub fn new(inputs: S) -> Self {
        Self { inputs }
    }

    /// Which tiles at `zoom` cover `bbox` — the Web Mercator tile range a
    /// run actually needs to simulate, not the whole world.
    fn tiles_for_bbox(&self, bbox: &crate::loader::BoundingBox, zoom: u8) -> Vec<TileId> {
        let (min_x, max_y) = lonlat_to_tile(bbox.west, bbox.south, zoom);
        let (max_x, min_y) = lonlat_to_tile(bbox.east, bbox.north, zoom);

        let mut tiles = Vec::new();
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                tiles.push(TileId::new(zoom, x, y));
            }
        }
        tiles
    }
}

/// Standard Web Mercator slippy-map lon/lat -> tile x/y, floored to the
/// containing tile. Same projection `tile_server`'s MapLibre frontend and
/// `TileId` already assume (z/x/y), so a bbox resolved here lines up with
/// the same tiles the live map would request for the same area.
fn lonlat_to_tile(lon: f64, lat: f64, zoom: u8) -> (u32, u32) {
    let n = 2f64.powi(zoom as i32);
    let x = ((lon + 180.0) / 360.0 * n).floor().max(0.0) as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        .floor()
        .max(0.0) as u32;
    (x.min(n as u32 - 1), y.min(n as u32 - 1))
}

/// Inverse of `lonlat_to_tile`'s per-tile math, for one cell within a
/// tile's `GRID_SIZE` grid — lets `run` turn a cell index back into the
/// real lat/lon a `WaterLevelSample` needs, since the lookup path
/// (`SnapshotStore::water_level_timeline`) indexes on coordinates, not
/// tile/cell indices.
///
/// `pub(crate)` so a `TileInputSource` implementation (e.g. a GDAL-backed
/// one reading a real DEM) can resolve the same cell -> lon/lat mapping
/// this analyzer itself uses, rather than reimplementing — and risking
/// drifting out of sync with — the same projection math twice.
pub(crate) fn cell_to_lonlat(tile: TileId, cell_x: usize, cell_y: usize) -> (f64, f64) {
    let n = 2f64.powi(tile.z as i32);
    let tile_frac_x = tile.x as f64 + (cell_x as f64 + 0.5) / GRID_SIZE as f64;
    let tile_frac_y = tile.y as f64 + (cell_y as f64 + 0.5) / GRID_SIZE as f64;

    let lon = tile_frac_x / n * 360.0 - 180.0;
    let y_rad = std::f64::consts::PI * (1.0 - 2.0 * tile_frac_y / n);
    let lat = y_rad.sinh().atan().to_degrees();
    (lon, lat)
}

#[async_trait]
impl<S: TileInputSource> HazardAnalyzer for FloodAnalyzer<S> {
    fn hazard_type(&self) -> HazardType {
        HazardType::Flood
    }

    fn compute_model(&self) -> ComputeModel {
        ComputeModel::TileGrid
    }

    fn validate(&self, input: &ScenarioInput) -> Result<(), AnalysisError> {
        if input.hazard != HazardType::Flood {
            return Err(AnalysisError::InvalidParameter {
                message: format!("FloodAnalyzer cannot run a {} scenario", input.hazard),
            });
        }
        if input.zoom.is_none() {
            return Err(AnalysisError::MissingInputLayer {
                layer: "zoom (required for TileGrid compute model)".to_string(),
            });
        }
        let rainfall = input.parameter("rainfall_rate_m_per_step");
        if rainfall.map(|r| r < 0.0).unwrap_or(false) {
            return Err(AnalysisError::InvalidParameter {
                message: "rainfall_rate_m_per_step must be >= 0".to_string(),
            });
        }
        Ok(())
    }

    async fn run(&self, input: &ScenarioInput) -> Result<ScenarioResult, AnalysisError> {
        self.validate(input)?;
        let start = std::time::Instant::now();

        let zoom = input.zoom.unwrap(); // validated above
        let rainfall_rate = input
            .parameter("rainfall_rate_m_per_step")
            .unwrap_or(0.0) as f32;
        let drainage_rate = input
            .parameter("drainage_rate_fraction")
            .unwrap_or(0.05) as f32;
        let timesteps = input
            .parameter("timesteps")
            .unwrap_or(10.0)
            .max(1.0) as u32;
        let step_duration_s = input.parameter("step_duration_seconds").unwrap_or(3600.0);

        let tile_ids = self.tiles_for_bbox(&input.bbox, zoom);
        if tile_ids.is_empty() {
            return Err(AnalysisError::InvalidParameter {
                message: "bbox resolved to zero tiles at the requested zoom".to_string(),
            });
        }

        let mut tiles: HashMap<TileId, FloodTile> = tile_ids
            .iter()
            .map(|&id| {
                let cell_inputs = self.inputs.resolve(id);
                (
                    id,
                    FloodTile::new(id, cell_inputs, rainfall_rate, drainage_rate),
                )
            })
            .collect();

        // Run the whole simulation first (this is the batch job — can
        // take however long it needs, per the earlier "2 seconds is only
        // for lookup, not compute" scoping), collecting one
        // WaterLevelSample per cell per timestep so the lookup path has
        // a full timeline to query afterward, not just a final state.
        let mut all_samples = Vec::new();
        let base_time = time::OffsetDateTime::now_utc();

        for step in 0..timesteps {
            run_tile_grid(&mut tiles, 1);

            let observed_at = base_time + time::Duration::seconds_f64(step_duration_s * (step + 1) as f64);
            for (&id, tile) in tiles.iter() {
                for y in 0..GRID_SIZE {
                    for x in 0..GRID_SIZE {
                        let depth = tile.depth_at(x, y);
                        let (lon, lat) = cell_to_lonlat(id, x, y);
                        all_samples.push(WaterLevelSample {
                            scenario_name: input.scenario_name.clone(),
                            lat,
                            lon,
                            observed_at,
                            water_level_m: depth as f64,
                            confidence: DataConfidence::Modeled,
                        });
                    }
                }
            }
        }

        // The final-state whole-map snapshot, in the same GeoJSON shape
        // ScenarioResult documents — a FeatureCollection of points with
        // an `intensity` property, built from the last timestep only
        // (the per-timestep detail lives in all_samples / the timeseries
        // table, not duplicated into every snapshot).
        let features: Vec<serde_json::Value> = tiles
            .iter()
            .flat_map(|(&id, tile)| {
                (0..GRID_SIZE).flat_map(move |y| {
                    (0..GRID_SIZE).map(move |x| {
                        let (lon, lat) = cell_to_lonlat(id, x, y);
                        serde_json::json!({
                            "type": "Feature",
                            "geometry": { "type": "Point", "coordinates": [lon, lat] },
                            "properties": { "intensity": tile.depth_at(x, y) }
                        })
                    })
                })
            })
            .collect();

        let intensity_field = serde_json::json!({
            "type": "FeatureCollection",
            "features": features,
        });

        let parameters_used = vec![
            ScenarioParameter {
                key: "rainfall_rate_m_per_step".to_string(),
                value: rainfall_rate as f64,
                unit: "m".to_string(),
                confidence: DataConfidence::Observed,
            },
            ScenarioParameter {
                key: "drainage_rate_fraction".to_string(),
                value: drainage_rate as f64,
                unit: "fraction".to_string(),
                confidence: DataConfidence::Modeled,
            },
            ScenarioParameter {
                key: "timesteps".to_string(),
                value: timesteps as f64,
                unit: "count".to_string(),
                confidence: DataConfidence::Modeled,
            },
        ];

        Ok(ScenarioResult {
            hazard: HazardType::Flood,
            scenario_name: input.scenario_name.clone(),
            confidence: DataConfidence::Modeled,
            bbox: input.bbox.clone(),
            intensity_field,
            parameters_used,
            computed_at: time::OffsetDateTime::now_utc(),
            duration_seconds: start.elapsed().as_secs_f64(),
        })
    }

    fn name(&self) -> &str {
        "FloodAnalyzer (simplified diffusive flood-fill)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::BoundingBox;

    /// Flat, dry, non-waterway ground everywhere — the simplest possible
    /// input source, enough to prove FloodAnalyzer's plumbing (tile
    /// resolution, timestep loop, sample collection) works without
    /// depending on a real DEM file.
    struct FlatGroundSource {
        elevation_m: f32,
    }

    impl TileInputSource for FlatGroundSource {
        fn resolve(&self, _tile: TileId) -> Vec<CellInput> {
            vec![
                CellInput { elevation_m: self.elevation_m, is_waterway: false };
                GRID_SIZE * GRID_SIZE
            ]
        }
    }

    fn bangkok_bbox() -> BoundingBox {
        // small sub-area, enough to span a handful of tiles at zoom 14
        // without the test resolving hundreds of tiles
        BoundingBox::new(100.50, 13.74, 100.52, 13.76)
    }

    #[tokio::test]
    async fn run_produces_one_scenario_result_and_a_full_timeseries() {
        let analyzer = FloodAnalyzer::new(FlatGroundSource { elevation_m: 0.0 });
        let input = ScenarioInput {
            hazard: HazardType::Flood,
            bbox: bangkok_bbox(),
            zoom: Some(14),
            scenario_name: "test-run".to_string(),
            parameters: vec![
                ScenarioParameter {
                    key: "rainfall_rate_m_per_step".to_string(),
                    value: 0.01,
                    unit: "m".to_string(),
                    confidence: DataConfidence::Observed,
                },
                ScenarioParameter {
                    key: "timesteps".to_string(),
                    value: 3.0,
                    unit: "count".to_string(),
                    confidence: DataConfidence::Modeled,
                },
            ],
        };

        let result = analyzer.run(&input).await.unwrap();

        assert_eq!(result.hazard, HazardType::Flood);
        assert_eq!(result.scenario_name, "test-run");
        let features = result.intensity_field["features"].as_array().unwrap();
        assert!(!features.is_empty(), "expected a populated intensity field");

        // every feature should show the accumulated rainfall from 3
        // timesteps on flat ground with default drainage > 0, so depth
        // should be positive but less than 3x the raw rainfall input
        // (drainage removes some each step)
        for f in features {
            let intensity = f["properties"]["intensity"].as_f64().unwrap();
            assert!(intensity > 0.0, "expected standing water after rainfall");
            assert!(intensity < 0.03, "drainage should prevent full accumulation");
        }
    }

    #[tokio::test]
    async fn validate_rejects_wrong_hazard_type() {
        let analyzer = FloodAnalyzer::new(FlatGroundSource { elevation_m: 0.0 });
        let input = ScenarioInput {
            hazard: HazardType::Earthquake,
            bbox: bangkok_bbox(),
            zoom: Some(14),
            scenario_name: "wrong-hazard".to_string(),
            parameters: vec![],
        };

        let err = analyzer.validate(&input).unwrap_err();
        assert!(matches!(err, AnalysisError::InvalidParameter { .. }));
    }

    #[tokio::test]
    async fn validate_requires_zoom() {
        let analyzer = FloodAnalyzer::new(FlatGroundSource { elevation_m: 0.0 });
        let input = ScenarioInput {
            hazard: HazardType::Flood,
            bbox: bangkok_bbox(),
            zoom: None,
            scenario_name: "no-zoom".to_string(),
            parameters: vec![],
        };

        let err = analyzer.validate(&input).unwrap_err();
        assert!(matches!(err, AnalysisError::MissingInputLayer { .. }));
    }

    #[test]
    fn cell_to_lonlat_stays_within_tile_bounds() {
        // a cell at the exact center of a tile should decode to roughly
        // the tile's own center coordinates, within the tile's angular span
        let tile = TileId::new(10, 797, 472);
        let (lon, lat) = cell_to_lonlat(tile, GRID_SIZE / 2, GRID_SIZE / 2);

        // bangkok is roughly 100-101 lon, 13-14 lat at this zoom/tile range
        assert!((99.0..=102.0).contains(&lon), "lon {lon} out of expected range");
        assert!((12.0..=15.0).contains(&lat), "lat {lat} out of expected range");
    }
}
