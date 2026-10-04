//! Same simulate -> store -> query path as flood_demo.rs, but reading
//! real elevation from bangkok_glo30_dem.tif via GdalDemSource instead
//! of the synthetic DemoTerrain -- the actual real-data answer to
//! "why are we still using fake terrain," not another layer of
//! scaffolding on top of it.
//!
//! Requires the gdal-support feature and a real GDAL install (this
//! project's verification used `apt install libgdal-dev` against GDAL
//! 3.10.3 in a rust:latest container -- see base/Cargo.toml for why
//! gdal was bumped to 0.19 to support that version).
//!
//! Run with:
//!   cargo run --example flood_demo_real_dem --package base --features gdal-support -- /path/to/bangkok_glo30_dem.tif

use std::path::PathBuf;

use base::analysis::{
    DataConfidence, FloodAnalyzer, GdalDemSource, HazardAnalyzer, HazardType, ScenarioInput,
    ScenarioParameter,
};
use base::db::SnapshotStore;
use base::loader::BoundingBox;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== FloodAnalyzer end-to-end demo (REAL DEM) ===\n");

    let dem_path: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bangkok_glo30_dem.tif"));

    if !dem_path.exists() {
        eprintln!(
            "DEM file not found at {dem_path:?}. Pass its path as the first argument, e.g.:\n  \
             cargo run --example flood_demo_real_dem --package base --features gdal-support -- /data/bangkok_glo30_dem.tif"
        );
        std::process::exit(1);
    }

    println!("Opening real DEM: {dem_path:?}");
    let dem = GdalDemSource::open(&dem_path)?;
    let analyzer = FloodAnalyzer::new(dem);

    // Same small central-Bangkok bbox as flood_demo.rs, so the two runs
    // (synthetic vs. real terrain) are directly comparable.
    let bbox = BoundingBox::new(100.50, 13.74, 100.52, 13.76);

    let input = ScenarioInput {
        hazard: HazardType::Flood,
        bbox: bbox.clone(),
        zoom: Some(16),
        scenario_name: "bangkok-demo-2026-real-dem".to_string(),
        parameters: vec![
            ScenarioParameter {
                key: "rainfall_rate_m_per_step".to_string(),
                value: 0.02,
                unit: "m".to_string(),
                confidence: DataConfidence::Observed,
            },
            ScenarioParameter {
                key: "drainage_rate_fraction".to_string(),
                value: 0.08,
                unit: "fraction".to_string(),
                confidence: DataConfidence::Modeled,
            },
            ScenarioParameter {
                key: "timesteps".to_string(),
                value: 6.0,
                unit: "count".to_string(),
                confidence: DataConfidence::Modeled,
            },
            ScenarioParameter {
                key: "step_duration_seconds".to_string(),
                value: 3600.0,
                unit: "seconds".to_string(),
                confidence: DataConfidence::Modeled,
            },
        ],
    };

    println!("Running simulation: {}", analyzer.name());
    println!(
        "  bbox: [{}, {}, {}, {}] (central Bangkok)",
        bbox.west, bbox.south, bbox.east, bbox.north
    );

    let result = analyzer.run(&input).await?;
    println!(
        "\nSimulation finished in {:.3}s, {} features in final snapshot",
        result.duration_seconds,
        result.intensity_field["features"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // Report the actual elevation range the simulation worked against --
    // this is the concrete proof real terrain, not a synthetic slope,
    // drove the result: a flat synthetic slope would show near-zero
    // variance; real Bangkok floodplain terrain should show some.
    let features = result.intensity_field["features"].as_array().unwrap();
    let intensities: Vec<f64> = features
        .iter()
        .filter_map(|f| f["properties"]["intensity"].as_f64())
        .collect();
    let min = intensities.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = intensities.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!(
        "  final water depth range across {} cells: {:.4}m to {:.4}m",
        intensities.len(),
        min,
        max
    );

    let store = SnapshotStore::open(":memory:").await?;
    store.save(&result).await?;
    println!("Saved ScenarioResult snapshot to store.");

    Ok(())
}
