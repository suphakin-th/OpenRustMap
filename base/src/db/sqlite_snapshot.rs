use snafu::ResultExt;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::FromRow;

use crate::analysis::{DataConfidence, HazardType, ScenarioResult, WaterLevelSample};
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

/// On-disk shape of one `water_level_timeseries` row — same rationale as
/// `SnapshotRow`: `confidence` is TEXT in SQLite, a typed enum in memory.
#[derive(Debug, FromRow)]
struct TimeseriesRow {
    scenario_name: String,
    lat: f64,
    lon: f64,
    observed_at: String,
    water_level_m: f64,
    confidence: String,
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

-- One row per (location, timestep) sample from a completed simulation run.
-- This is the table the <2s lookup path queries: "what was the water level
-- at this point, over this time range" is an index range-scan here, never a
-- GeoJSON parse or a live simulation run. scenario_snapshots above stays the
-- whole-map-at-one-instant record; this table is the point-over-time record
-- a run also produces — the same run writes to both, each serving a
-- different question.
CREATE TABLE IF NOT EXISTS water_level_timeseries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scenario_name   TEXT NOT NULL,
    -- Rounded to ~1m precision (5 decimal places) so repeated queries at a
    -- "nearby enough" point hit the same index range instead of requiring
    -- a spatial-distance calculation per row.
    lat             REAL NOT NULL,
    lon             REAL NOT NULL,
    observed_at     TEXT NOT NULL,   -- RFC3339; simulated timestamp, not wall-clock insert time
    water_level_m   REAL NOT NULL,
    confidence      TEXT NOT NULL
);

-- Covers the exact query shape the lookup path runs: fixed point, time range.
CREATE INDEX IF NOT EXISTS idx_timeseries_point_time
    ON water_level_timeseries (lat, lon, observed_at);
CREATE INDEX IF NOT EXISTS idx_timeseries_scenario
    ON water_level_timeseries (scenario_name);
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

    /// Write every sample from one finished simulation run in a single
    /// transaction, batched in chunks of `BATCH_SIZE` rows per `INSERT`.
    ///
    /// This is the actual performance lever for the write side — SQLite's
    /// cost is dominated by the number of separate statements/transactions,
    /// not by total row count, so one transaction with N rows per INSERT is
    /// what makes "write a whole tile-grid run's worth of samples" fast;
    /// looping `save` row-by-row would do one fsync per row and dominate
    /// the whole batch job's runtime for no benefit, since nothing here
    /// needs per-row durability mid-batch — only the finished batch does.
    ///
    /// Call this once per completed run, not incrementally per timestep:
    /// a `HazardAnalyzer::run` that computes a tile grid across many
    /// timesteps should accumulate samples in memory and write them here
    /// after the run finishes, exactly like it does for `ScenarioResult`
    /// via `save` above.
    pub async fn save_timeseries_batch(&self, samples: &[WaterLevelSample]) -> Result<(), Error> {
        // SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 999 bound
        // parameters per statement. Each row here binds 6 columns, so the
        // batch size must stay at 999/6 = 166 or fewer rows per INSERT —
        // this is the same class of bug as ADR-002 in the project README
        // (Postgres's u16::MAX param limit silently exceeded by a
        // too-large QueryBuilder batch), just a much lower ceiling because
        // SQLite's default is far stricter than Postgres's.
        const BATCH_SIZE: usize = 150;

        let mut tx = self.pool.begin().await.map_err(|e| Error::DatabaseError {
            message: format!("failed to start timeseries batch transaction: {}", e),
        })?;

        for chunk in samples.chunks(BATCH_SIZE) {
            let mut qb = sqlx::QueryBuilder::new(
                "INSERT INTO water_level_timeseries \
                 (scenario_name, lat, lon, observed_at, water_level_m, confidence) ",
            );
            qb.push_values(chunk, |mut row, sample| {
                let observed_at = sample
                    .observed_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default();
                row.push_bind(&sample.scenario_name)
                    .push_bind(sample.lat)
                    .push_bind(sample.lon)
                    .push_bind(observed_at)
                    .push_bind(sample.water_level_m)
                    .push_bind(confidence_str(sample.confidence));
            });

            qb.build()
                .execute(&mut *tx)
                .await
                .map_err(|e| Error::DatabaseError {
                    message: format!("failed to insert timeseries batch: {}", e),
                })?;
        }

        tx.commit().await.map_err(|e| Error::DatabaseError {
            message: format!("failed to commit timeseries batch: {}", e),
        })?;

        Ok(())
    }

    /// The <2s lookup path: water level at one point, over a time range,
    /// read from already-computed samples — never runs a simulation.
    ///
    /// `lat`/`lon` match on their rounded stored value (see the schema
    /// comment on `water_level_timeseries`), so pass the same rounding
    /// the writer used, not an arbitrary nearby coordinate — this is an
    /// exact index match, not a spatial nearest-neighbor search, which is
    /// what keeps it fast enough to stay under the 2-second budget without
    /// needing a spatial index extension.
    pub async fn water_level_timeline(
        &self,
        scenario_name: &str,
        lat: f64,
        lon: f64,
        from: time::OffsetDateTime,
        to: time::OffsetDateTime,
    ) -> Result<Vec<WaterLevelSample>, Error> {
        let from_str = from
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| Error::NaiveDateTimeError)?;
        let to_str = to
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| Error::NaiveDateTimeError)?;

        let rows: Vec<TimeseriesRow> = sqlx::query_as(
            "SELECT scenario_name, lat, lon, observed_at, water_level_m, confidence \
             FROM water_level_timeseries \
             WHERE scenario_name = ? AND lat = ? AND lon = ? \
               AND observed_at BETWEEN ? AND ? \
             ORDER BY observed_at ASC",
        )
        .bind(scenario_name)
        .bind(lat)
        .bind(lon)
        .bind(&from_str)
        .bind(&to_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to query water level timeline: {}", e),
        })?;

        rows.into_iter().map(timeseries_row_to_sample).collect()
    }
}

fn confidence_str(c: DataConfidence) -> &'static str {
    match c {
        DataConfidence::Observed => "observed",
        DataConfidence::Modeled => "modeled",
        DataConfidence::Hypothetical => "hypothetical",
    }
}

fn confidence_from_str(s: &str) -> Result<DataConfidence, Error> {
    match s {
        "observed" => Ok(DataConfidence::Observed),
        "modeled" => Ok(DataConfidence::Modeled),
        "hypothetical" => Ok(DataConfidence::Hypothetical),
        other => Err(Error::DatabaseError {
            message: format!("unknown confidence in snapshot store: {other}"),
        }),
    }
}

fn hazard_from_str(s: &str) -> Result<HazardType, Error> {
    match s {
        "flood" => Ok(HazardType::Flood),
        "earthquake" => Ok(HazardType::Earthquake),
        "wildfire" => Ok(HazardType::Wildfire),
        "asteroid_impact" => Ok(HazardType::AsteroidImpact),
        other => Err(Error::DatabaseError {
            message: format!("unknown hazard_type in snapshot store: {other}"),
        }),
    }
}

fn row_to_result(row: SnapshotRow) -> Result<ScenarioResult, Error> {
    use crate::loader::BoundingBox;

    let hazard = hazard_from_str(&row.hazard_type)?;
    let confidence = confidence_from_str(&row.confidence)?;

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

fn timeseries_row_to_sample(row: TimeseriesRow) -> Result<WaterLevelSample, Error> {
    Ok(WaterLevelSample {
        scenario_name: row.scenario_name,
        lat: row.lat,
        lon: row.lon,
        observed_at: time::OffsetDateTime::parse(
            &row.observed_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| Error::NaiveDateTimeError)?,
        water_level_m: row.water_level_m,
        confidence: confidence_from_str(&row.confidence)?,
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

    fn sample_at(scenario: &str, hour: i64, level_m: f64) -> WaterLevelSample {
        WaterLevelSample {
            scenario_name: scenario.to_string(),
            lat: 13.75,
            lon: 100.5,
            observed_at: time::OffsetDateTime::from_unix_timestamp(1_790_000_000 + hour * 3600)
                .unwrap(),
            water_level_m: level_m,
            confidence: DataConfidence::Modeled,
        }
    }

    #[tokio::test]
    async fn timeline_returns_samples_in_time_order() {
        let store = store().await;
        // insert out of order to confirm the query orders them, not the insert
        store
            .save_timeseries_batch(&[
                sample_at("bkk-flood", 2, 0.8),
                sample_at("bkk-flood", 0, 0.1),
                sample_at("bkk-flood", 1, 0.4),
            ])
            .await
            .unwrap();

        let from = time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
        let to = time::OffsetDateTime::from_unix_timestamp(1_790_000_000 + 3 * 3600).unwrap();
        let timeline = store
            .water_level_timeline("bkk-flood", 13.75, 100.5, from, to)
            .await
            .unwrap();

        assert_eq!(timeline.len(), 3);
        assert_eq!(
            timeline.iter().map(|s| s.water_level_m).collect::<Vec<_>>(),
            vec![0.1, 0.4, 0.8]
        );
    }

    #[tokio::test]
    async fn timeline_excludes_samples_outside_requested_range() {
        let store = store().await;
        store
            .save_timeseries_batch(&[
                sample_at("bkk-flood", 0, 0.1),
                sample_at("bkk-flood", 5, 0.9), // outside the 0..=2h window below
            ])
            .await
            .unwrap();

        let from = time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
        let to = time::OffsetDateTime::from_unix_timestamp(1_790_000_000 + 2 * 3600).unwrap();
        let timeline = store
            .water_level_timeline("bkk-flood", 13.75, 100.5, from, to)
            .await
            .unwrap();

        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].water_level_m, 0.1);
    }

    #[tokio::test]
    async fn batch_larger_than_sqlite_param_limit_still_writes_every_row() {
        // 300 rows x 6 columns = 1800 bound params, which exceeds SQLite's
        // default 999-parameter ceiling if sent as one INSERT — this proves
        // save_timeseries_batch's internal chunking actually avoids that.
        let store = store().await;
        let samples: Vec<WaterLevelSample> = (0..300)
            .map(|h| sample_at("bulk-test", h, h as f64 * 0.01))
            .collect();

        store.save_timeseries_batch(&samples).await.unwrap();

        let from = time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
        let to = time::OffsetDateTime::from_unix_timestamp(1_790_000_000 + 400 * 3600).unwrap();
        let timeline = store
            .water_level_timeline("bulk-test", 13.75, 100.5, from, to)
            .await
            .unwrap();

        assert_eq!(timeline.len(), 300);
    }
}
