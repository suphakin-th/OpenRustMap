use snafu::ResultExt;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::FromRow;

use crate::analysis::{
    BlendedReading, BlendedSource, DataConfidence, FieldReport, HazardType, ReportSeverity,
    ScenarioResult, WaterLevelBucket, WaterLevelSample,
};
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

/// On-disk shape of one `field_reports` row — same rationale as the rows
/// above: `water_level`/`severity` are TEXT in SQLite, typed enums in
/// memory.
#[derive(Debug, FromRow)]
struct FieldReportRow {
    id: i64,
    lat: f64,
    lon: f64,
    water_level: String,
    severity: String,
    note: Option<String>,
    photo_url: Option<String>,
    reported_at: String,
    confirm_count: i64,
    dispute_count: i64,
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

-- One row per person's real-time report of conditions at a location — the
-- crowd/ground-truth half of the aggregation this project builds toward,
-- deliberately a separate table from water_level_timeseries (simulated,
-- regular-grid, always-precise) rather than a shared schema with nullable
-- columns for whichever kind doesn't apply. See FieldReport's own doc
-- comment for the full reasoning.
CREATE TABLE IF NOT EXISTS field_reports (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    lat             REAL NOT NULL,
    lon             REAL NOT NULL,
    water_level     TEXT NOT NULL,   -- WaterLevelBucket: dry/ankle/knee/waist/chest
    severity        TEXT NOT NULL,   -- ReportSeverity: safe/caution/dangerous
    note            TEXT,
    photo_url       TEXT,
    reported_at     TEXT NOT NULL,   -- RFC3339
    confirm_count   INTEGER NOT NULL DEFAULT 0,
    dispute_count   INTEGER NOT NULL DEFAULT 0
);

-- Reports aren't on a regular grid like simulated samples, so a lookup
-- can't do an exact lat/lon match the way water_level_timeline does — a
-- bbox range scan (see nearby_field_reports) is the realistic query shape,
-- and this index covers exactly that: lat range first (the outer bound of
-- a small bbox query), then lon, then recency so "most recent nearby
-- reports" doesn't need a separate sort pass.
CREATE INDEX IF NOT EXISTS idx_field_reports_location
    ON field_reports (lat, lon, reported_at);
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

    /// Submit one person's report. Returns it with `id` filled in — the
    /// caller needs that id to later call `confirm_report`/`dispute_report`
    /// against this exact row.
    pub async fn submit_report(&self, report: &FieldReport) -> Result<FieldReport, Error> {
        let reported_at = report
            .reported_at
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| Error::NaiveDateTimeError)?;

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO field_reports \
                (lat, lon, water_level, severity, note, photo_url, reported_at, confirm_count, dispute_count) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0) \
             RETURNING id",
        )
        .bind(report.lat)
        .bind(report.lon)
        .bind(water_level_str(report.water_level))
        .bind(severity_str(report.severity))
        .bind(&report.note)
        .bind(&report.photo_url)
        .bind(&reported_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to submit field report: {}", e),
        })?;

        Ok(FieldReport {
            id: Some(id),
            ..report.clone()
        })
    }

    /// Someone on the ground confirms a report still matches what they
    /// see — raises its `trust_score` (see `FieldReport::trust_score`).
    pub async fn confirm_report(&self, report_id: i64) -> Result<(), Error> {
        sqlx::query("UPDATE field_reports SET confirm_count = confirm_count + 1 WHERE id = ?")
            .bind(report_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DatabaseError {
                message: format!("failed to confirm report {report_id}: {e}"),
            })?;
        Ok(())
    }

    /// Someone on the ground disputes a report (K.A.R.A.'s "Water gone" /
    /// no-longer-accurate signal) — lowers its `trust_score`.
    pub async fn dispute_report(&self, report_id: i64) -> Result<(), Error> {
        sqlx::query("UPDATE field_reports SET dispute_count = dispute_count + 1 WHERE id = ?")
            .bind(report_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DatabaseError {
                message: format!("failed to dispute report {report_id}: {e}"),
            })?;
        Ok(())
    }

    /// Every report within a small lat/lon box around `(lat, lon)`,
    /// newest first. `radius_deg` is a plain coordinate-degree box, not a
    /// true geodesic radius — deliberately: at the scale this is used for
    /// (a few hundred metres around one point, within Bangkok's latitude),
    /// the distortion from treating degrees as flat is far smaller than
    /// the uncertainty already inherent in a person's own location fix,
    /// so a real haversine calculation would add complexity without
    /// adding accuracy that matters here.
    pub async fn nearby_field_reports(
        &self,
        lat: f64,
        lon: f64,
        radius_deg: f64,
    ) -> Result<Vec<FieldReport>, Error> {
        let rows: Vec<FieldReportRow> = sqlx::query_as(
            "SELECT * FROM field_reports \
             WHERE lat BETWEEN ? AND ? AND lon BETWEEN ? AND ? \
             ORDER BY reported_at DESC",
        )
        .bind(lat - radius_deg)
        .bind(lat + radius_deg)
        .bind(lon - radius_deg)
        .bind(lon + radius_deg)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DatabaseError {
            message: format!("failed to query nearby field reports: {}", e),
        })?;

        rows.into_iter().map(field_report_row_to_report).collect()
    }

    /// The actual blended answer to "how deep is the water here, right
    /// now" — this is the aggregation step the whole field_reports
    /// addition exists for, standing in for the "weighs" / "aggregates"
    /// stages of a pipeline like K.A.R.A.'s own.
    ///
    /// Preference order, deliberately simple and explainable rather than
    /// a black-box score:
    /// 1. The most recent field report with `trust_score() > 0` — a
    ///    corroborated eyewitness beats a model every time they disagree,
    ///    because the model is an estimate and the report is what
    ///    actually happened.
    /// 2. The most recent field report at all, if none are corroborated
    ///    yet — still better than nothing, but flagged as such by the
    ///    caller checking `trust_score()` itself if it needs to know.
    /// 3. The simulated value for the same point, if no report exists
    ///    nearby at all.
    ///
    /// Returns `None` only when neither a report nor a simulated sample
    /// covers this point — i.e. this location truly has no data yet.
    pub async fn blended_water_level(
        &self,
        scenario_name: &str,
        lat: f64,
        lon: f64,
        radius_deg: f64,
        as_of: time::OffsetDateTime,
    ) -> Result<Option<BlendedReading>, Error> {
        let reports = self.nearby_field_reports(lat, lon, radius_deg).await?;

        if let Some(trusted) = reports.iter().find(|r| r.trust_score() > 0) {
            return Ok(Some(BlendedReading {
                water_level_m: trusted.water_level.approx_m(),
                source: BlendedSource::FieldReport,
                confidence: DataConfidence::Observed,
                as_of: trusted.reported_at,
            }));
        }
        if let Some(unconfirmed) = reports.first() {
            return Ok(Some(BlendedReading {
                water_level_m: unconfirmed.water_level.approx_m(),
                source: BlendedSource::UnconfirmedFieldReport,
                confidence: DataConfidence::Observed,
                as_of: unconfirmed.reported_at,
            }));
        }

        // fall back to the nearest-in-time simulated sample at this exact
        // point — reuses water_level_timeline's exact-match lookup with a
        // narrow window around `as_of` rather than a fresh query shape.
        let window_before = as_of - time::Duration::hours(1);
        let window_after = as_of + time::Duration::hours(1);
        let samples = self
            .water_level_timeline(scenario_name, lat, lon, window_before, window_after)
            .await?;

        let closest = samples.into_iter().min_by_key(|s| {
            (s.observed_at - as_of).whole_seconds().unsigned_abs()
        });

        Ok(closest.map(|s| BlendedReading {
            water_level_m: s.water_level_m,
            source: BlendedSource::Simulated,
            confidence: s.confidence,
            as_of: s.observed_at,
        }))
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

fn water_level_str(w: WaterLevelBucket) -> &'static str {
    match w {
        WaterLevelBucket::Dry => "dry",
        WaterLevelBucket::Ankle => "ankle",
        WaterLevelBucket::Knee => "knee",
        WaterLevelBucket::Waist => "waist",
        WaterLevelBucket::Chest => "chest",
    }
}

fn water_level_from_str(s: &str) -> Result<WaterLevelBucket, Error> {
    match s {
        "dry" => Ok(WaterLevelBucket::Dry),
        "ankle" => Ok(WaterLevelBucket::Ankle),
        "knee" => Ok(WaterLevelBucket::Knee),
        "waist" => Ok(WaterLevelBucket::Waist),
        "chest" => Ok(WaterLevelBucket::Chest),
        other => Err(Error::DatabaseError {
            message: format!("unknown water_level in field_reports: {other}"),
        }),
    }
}

fn severity_str(s: ReportSeverity) -> &'static str {
    match s {
        ReportSeverity::Safe => "safe",
        ReportSeverity::Caution => "caution",
        ReportSeverity::Dangerous => "dangerous",
    }
}

fn severity_from_str(s: &str) -> Result<ReportSeverity, Error> {
    match s {
        "safe" => Ok(ReportSeverity::Safe),
        "caution" => Ok(ReportSeverity::Caution),
        "dangerous" => Ok(ReportSeverity::Dangerous),
        other => Err(Error::DatabaseError {
            message: format!("unknown severity in field_reports: {other}"),
        }),
    }
}

fn field_report_row_to_report(row: FieldReportRow) -> Result<FieldReport, Error> {
    Ok(FieldReport {
        id: Some(row.id),
        lat: row.lat,
        lon: row.lon,
        water_level: water_level_from_str(&row.water_level)?,
        severity: severity_from_str(&row.severity)?,
        note: row.note,
        photo_url: row.photo_url,
        reported_at: time::OffsetDateTime::parse(
            &row.reported_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| Error::NaiveDateTimeError)?,
        confirm_count: row.confirm_count,
        dispute_count: row.dispute_count,
    })
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

    fn sample_report(lat: f64, lon: f64, level: WaterLevelBucket) -> FieldReport {
        FieldReport {
            id: None,
            lat,
            lon,
            water_level: level,
            severity: ReportSeverity::Caution,
            note: Some("test report".to_string()),
            photo_url: None,
            reported_at: time::OffsetDateTime::now_utc(),
            confirm_count: 0,
            dispute_count: 0,
        }
    }

    #[tokio::test]
    async fn submit_report_assigns_id_and_roundtrips() {
        let store = store().await;
        let report = sample_report(13.75, 100.51, WaterLevelBucket::Knee);

        let saved = store.submit_report(&report).await.unwrap();
        assert!(saved.id.is_some());

        let nearby = store
            .nearby_field_reports(13.75, 100.51, 0.001)
            .await
            .unwrap();
        assert_eq!(nearby.len(), 1);
        assert_eq!(nearby[0].water_level, WaterLevelBucket::Knee);
        assert_eq!(nearby[0].confirm_count, 0);
    }

    #[tokio::test]
    async fn confirm_and_dispute_change_trust_score() {
        let store = store().await;
        let saved = store
            .submit_report(&sample_report(13.75, 100.51, WaterLevelBucket::Waist))
            .await
            .unwrap();
        let id = saved.id.unwrap();

        store.confirm_report(id).await.unwrap();
        store.confirm_report(id).await.unwrap();
        store.dispute_report(id).await.unwrap();

        let nearby = store
            .nearby_field_reports(13.75, 100.51, 0.001)
            .await
            .unwrap();
        assert_eq!(nearby[0].confirm_count, 2);
        assert_eq!(nearby[0].dispute_count, 1);
        assert_eq!(nearby[0].trust_score(), 1); // 2 confirms - 1 dispute
    }

    #[tokio::test]
    async fn trust_score_never_goes_negative() {
        let store = store().await;
        let saved = store
            .submit_report(&sample_report(13.75, 100.51, WaterLevelBucket::Ankle))
            .await
            .unwrap();
        let id = saved.id.unwrap();

        store.dispute_report(id).await.unwrap();
        store.dispute_report(id).await.unwrap();
        store.dispute_report(id).await.unwrap();

        let nearby = store
            .nearby_field_reports(13.75, 100.51, 0.001)
            .await
            .unwrap();
        assert_eq!(nearby[0].trust_score(), 0); // floored, not -3
    }

    #[tokio::test]
    async fn nearby_field_reports_excludes_out_of_radius_points() {
        let store = store().await;
        store
            .submit_report(&sample_report(13.75, 100.51, WaterLevelBucket::Knee))
            .await
            .unwrap();
        store
            .submit_report(&sample_report(14.50, 101.20, WaterLevelBucket::Chest)) // far away
            .await
            .unwrap();

        let nearby = store
            .nearby_field_reports(13.75, 100.51, 0.01)
            .await
            .unwrap();
        assert_eq!(nearby.len(), 1);
        assert_eq!(nearby[0].water_level, WaterLevelBucket::Knee);
    }

    #[tokio::test]
    async fn blended_prefers_corroborated_report_over_unconfirmed_and_simulation() {
        let store = store().await;
        let now = time::OffsetDateTime::now_utc();

        // simulated baseline for this point
        store
            .save_timeseries_batch(&[WaterLevelSample {
                scenario_name: "bkk-blend-test".to_string(),
                lat: 13.75,
                lon: 100.51,
                observed_at: now,
                water_level_m: 0.05, // low — simulation thinks it's barely wet
                confidence: DataConfidence::Modeled,
            }])
            .await
            .unwrap();

        // an unconfirmed report disagreeing with the simulation
        let unconfirmed = store
            .submit_report(&sample_report(13.75, 100.51, WaterLevelBucket::Ankle))
            .await
            .unwrap();

        // blended should prefer the unconfirmed report over simulation,
        // since any real report outranks a pure model guess
        let reading = store
            .blended_water_level("bkk-blend-test", 13.75, 100.51, 0.01, now)
            .await
            .unwrap()
            .expect("expected a reading");
        assert_eq!(reading.source, BlendedSource::UnconfirmedFieldReport);
        assert_eq!(reading.water_level_m, WaterLevelBucket::Ankle.approx_m());

        // now corroborate a DIFFERENT, more severe report — this should
        // take over as the top answer
        let corroborated = store
            .submit_report(&sample_report(13.75, 100.51, WaterLevelBucket::Chest))
            .await
            .unwrap();
        store.confirm_report(corroborated.id.unwrap()).await.unwrap();
        let _ = unconfirmed; // keep alive for clarity; already persisted

        let reading = store
            .blended_water_level("bkk-blend-test", 13.75, 100.51, 0.01, now)
            .await
            .unwrap()
            .expect("expected a reading");
        assert_eq!(reading.source, BlendedSource::FieldReport);
        assert_eq!(reading.water_level_m, WaterLevelBucket::Chest.approx_m());
    }

    #[tokio::test]
    async fn blended_falls_back_to_simulation_when_no_reports_exist() {
        let store = store().await;
        let now = time::OffsetDateTime::now_utc();

        store
            .save_timeseries_batch(&[WaterLevelSample {
                scenario_name: "bkk-sim-only".to_string(),
                lat: 13.80,
                lon: 100.60,
                observed_at: now,
                water_level_m: 0.30,
                confidence: DataConfidence::Modeled,
            }])
            .await
            .unwrap();

        let reading = store
            .blended_water_level("bkk-sim-only", 13.80, 100.60, 0.01, now)
            .await
            .unwrap()
            .expect("expected a reading");
        assert_eq!(reading.source, BlendedSource::Simulated);
        assert_eq!(reading.water_level_m, 0.30);
    }

    #[tokio::test]
    async fn blended_returns_none_when_nothing_covers_the_point() {
        let store = store().await;
        let now = time::OffsetDateTime::now_utc();

        let reading = store
            .blended_water_level("no-such-scenario", 0.0, 0.0, 0.01, now)
            .await
            .unwrap();
        assert!(reading.is_none());
    }
}
