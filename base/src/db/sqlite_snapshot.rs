use snafu::ResultExt;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::FromRow;

use crate::analysis::{DataConfidence, HazardType, ScenarioResult};
use crate::error::{self, Error};

/// Row shape exactly as stored — kept separate from `ScenarioResult` because
/// the DB has no native JSON/enum types: `intensity_field`/`parameters_used`
/// are stored as TEXT and hazard/confidence as their string form, so this
/// struct is the honest on-disk shape and `row_to_result` does the one-time
/// decode into the richer in-memory type.
#[derive(Debug, FromRow)]
struct SnapshotRow {
    scenario_name: String,
    hazard_type: String,
    confidence: String,
    bbox_west: f64,
    bbox_south: f64,
    bbox_east: f64,
    bbox_north: f64,
    intensity_field: String,
    parameters_used: String,
    computed_at: String,
    duration_seconds: f64,
}

/// Embedded, file-based store for finished scenario results.
///
/// This is deliberately separate from the PostgreSQL/PostGIS pool in
/// `db::pool` — it exists so a run's *summary* can be opened and browsed
/// on a machine with nothing installed beyond the compiled binary, no
/// server to start, no network dependency. One `.db` file, one connection
/// pool, done.
///
/// What belongs here vs. in raw observed/modeled data on disk:
/// - Raw DEM tiles, OSM geometry, gauge readings: stay as GeoTIFF/COG and
///   GeoParquet files, read directly, never copied into this store.
/// - A finished `ScenarioResult` (a completed simulation run, with the
///   parameters that produced it and a timestamp): goes here, once, after
///   `HazardAnalyzer::run` returns — so re-opening a past flood/earthquake
///   scenario never means re-running the simulation.
pub struct SnapshotStore {
    pool: SqlitePool,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scenario_snapshots (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scenario_name   TEXT NOT NULL,
    hazard_type     TEXT NOT NULL,
    confidence      TEXT NOT NULL,
    bbox_west       REAL NOT NULL,
    bbox_south      REAL NOT NULL,
    bbox_east       REAL NOT NULL,
    bbox_north      REAL NOT NULL,
    intensity_field TEXT NOT NULL,   -- GeoJSON, stored as text (SQLite has no native JSON type)
    parameters_used TEXT NOT NULL,   -- JSON array of ScenarioParameter
    computed_at     TEXT NOT NULL,   -- RFC3339
    duration_seconds REAL NOT NULL,
    UNIQUE (scenario_name, hazard_type)
);

CREATE INDEX IF NOT EXISTS idx_snapshots_hazard ON scenario_snapshots (hazard_type);
CREATE INDEX IF NOT EXISTS idx_snapshots_computed_at ON scenario_snapshots (computed_at);
"#;

impl SnapshotStore {
    /// Open (creating if needed) a snapshot store at the given path.
    ///
    /// `sqlite://path/to/file.db?mode=rwc` — `rwc` creates the file on
    /// first use, matching what "no separate setup step" actually means.
    /// Pass `":memory:"` for a private in-memory DB (used by the tests
    /// below) — that's sqlx's own special-cased connection string, not a
    /// real file path, so it's routed around the `sqlite://...?mode=rwc`
    /// form rather than through it.
    pub async fn open(db_path: &str) -> Result<Self, Error> {
        let url = if db_path == ":memory:" {
            "sqlite::memory:".to_string()
        } else {
            format!("sqlite://{}?mode=rwc", db_path)
        };

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .map_err(|e| Error::DatabaseError {
                message: format!("failed to open snapshot store at {}: {}", db_path, e),
            })?;

        sqlx::query(SCHEMA)
            .execute(&pool)
            .await
            .map_err(|e| Error::DatabaseError {
                message: format!("failed to initialize snapshot store schema: {}", e),
            })?;

        Ok(Self { pool })
    }

    /// Persist a finished scenario result. Re-running the same
    /// `scenario_name` + hazard overwrites the previous snapshot rather
    /// than accumulating duplicates — the history you get is "the last
    /// time each named scenario was computed," not every attempt.
    pub async fn save(&self, result: &ScenarioResult) -> Result<(), Error> {
        let intensity_json =
            serde_json::to_string(&result.intensity_field).context(error::SerdeJsonSnafu)?;
        let params_json =
            serde_json::to_string(&result.parameters_used).context(error::SerdeJsonSnafu)?;
        let computed_at = result
            .computed_at
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| Error::NaiveDateTimeError)?;

        sqlx::query(
            r#"
            INSERT INTO scenario_snapshots
                (scenario_name, hazard_type, confidence, bbox_west, bbox_south, bbox_east, bbox_north,
                 intensity_field, parameters_used, computed_at, duration_seconds)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT (scenario_name, hazard_type) DO UPDATE SET
                confidence = excluded.confidence,
                bbox_west = excluded.bbox_west,
                bbox_south = excluded.bbox_south,
                bbox_east = excluded.bbox_east,
                bbox_north = excluded.bbox_north,
                intensity_field = excluded.intensity_field,
                parameters_used = excluded.parameters_used,
                computed_at = excluded.computed_at,
                duration_seconds = excluded.duration_seconds
            "#,
        )
        .bind(&result.scenario_name)
        .bind(result.hazard.to_string())
        .bind(confidence_str(result.confidence))
        .bind(result.bbox.west)
        .bind(result.bbox.south)
        .bind(result.bbox.east)
        .bind(result.bbox.north)
        .bind(&intensity_json)
        .bind(&params_json)
        .bind(&computed_at)
        .bind(result.duration_seconds)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to save snapshot: {}", e),
        })?;

        Ok(())
    }

    /// Look up a past run by name and hazard, without recomputing anything.
    pub async fn load(
        &self,
        scenario_name: &str,
        hazard: HazardType,
    ) -> Result<Option<ScenarioResult>, Error> {
        let row: Option<SnapshotRow> = sqlx::query_as(
            "SELECT * FROM scenario_snapshots WHERE scenario_name = ? AND hazard_type = ?",
        )
        .bind(scenario_name)
        .bind(hazard.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to load snapshot: {}", e),
        })?;

        row.map(row_to_result).transpose()
    }

    /// List every stored run for a hazard, most recent first — the
    /// "history" view: what's already been computed and can be opened
    /// instantly instead of rerun.
    pub async fn list(&self, hazard: HazardType) -> Result<Vec<ScenarioResult>, Error> {
        let rows: Vec<SnapshotRow> = sqlx::query_as(
            "SELECT * FROM scenario_snapshots WHERE hazard_type = ? ORDER BY computed_at DESC",
        )
        .bind(hazard.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to list snapshots: {}", e),
        })?;

        rows.into_iter().map(row_to_result).collect()
    }
}

fn confidence_str(c: DataConfidence) -> &'static str {
    match c {
        DataConfidence::Observed => "observed",
        DataConfidence::Modeled => "modeled",
        DataConfidence::Hypothetical => "hypothetical",
    }
}

fn row_to_result(row: SnapshotRow) -> Result<ScenarioResult, Error> {
    use crate::loader::BoundingBox;

    let hazard = match row.hazard_type.as_str() {
        "flood" => HazardType::Flood,
        "earthquake" => HazardType::Earthquake,
        "wildfire" => HazardType::Wildfire,
        "asteroid_impact" => HazardType::AsteroidImpact,
        other => {
            return Err(Error::DatabaseError {
                message: format!("unknown hazard_type in snapshot store: {other}"),
            })
        }
    };

    let confidence = match row.confidence.as_str() {
        "observed" => DataConfidence::Observed,
        "modeled" => DataConfidence::Modeled,
        "hypothetical" => DataConfidence::Hypothetical,
        other => {
            return Err(Error::DatabaseError {
                message: format!("unknown confidence in snapshot store: {other}"),
            })
        }
    };

    Ok(ScenarioResult {
        hazard,
        scenario_name: row.scenario_name,
        confidence,
        bbox: BoundingBox::new(row.bbox_west, row.bbox_south, row.bbox_east, row.bbox_north),
        intensity_field: serde_json::from_str(&row.intensity_field)
            .map_err(|e| Error::SerdeJsonError { source: e })?,
        parameters_used: serde_json::from_str(&row.parameters_used)
            .map_err(|e| Error::SerdeJsonError { source: e })?,
        computed_at: time::OffsetDateTime::parse(
            &row.computed_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| Error::NaiveDateTimeError)?,
        duration_seconds: row.duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ScenarioParameter;
    use crate::loader::BoundingBox;

    async fn store() -> SnapshotStore {
        // in-memory DB per test — no file, no cross-test interference
        SnapshotStore::open(":memory:").await.unwrap()
    }

    fn sample_result(name: &str) -> ScenarioResult {
        ScenarioResult {
            hazard: HazardType::Flood,
            scenario_name: name.to_string(),
            confidence: DataConfidence::Modeled,
            bbox: BoundingBox::new(100.33, 13.22, 100.94, 13.96),
            intensity_field: serde_json::json!({"type": "FeatureCollection", "features": []}),
            parameters_used: vec![ScenarioParameter {
                key: "rainfall_mm_24h".to_string(),
                value: 300.0,
                unit: "mm".to_string(),
                confidence: DataConfidence::Observed,
            }],
            computed_at: time::OffsetDateTime::now_utc(),
            duration_seconds: 12.5,
        }
    }

    #[tokio::test]
    async fn save_then_load_roundtrips() {
        let store = store().await;
        let result = sample_result("bangkok-sep2026-test");
        store.save(&result).await.unwrap();

        let loaded = store
            .load("bangkok-sep2026-test", HazardType::Flood)
            .await
            .unwrap()
            .expect("snapshot should exist");

        assert_eq!(loaded.scenario_name, result.scenario_name);
        assert_eq!(loaded.duration_seconds, result.duration_seconds);
    }

    #[tokio::test]
    async fn missing_scenario_returns_none() {
        let store = store().await;
        let loaded = store
            .load("does-not-exist", HazardType::Flood)
            .await
            .unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn rerunning_same_name_overwrites_not_duplicates() {
        let store = store().await;
        store.save(&sample_result("same-name")).await.unwrap();
        store.save(&sample_result("same-name")).await.unwrap();

        let all = store.list(HazardType::Flood).await.unwrap();
        assert_eq!(all.len(), 1);
    }
}
