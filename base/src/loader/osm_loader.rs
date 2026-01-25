use async_trait::async_trait;
use geo_types::{Geometry, LineString, Point, Polygon};
use osmpbfreader::{OsmObj, OsmPbfReader};
use snafu::ResultExt;
use sqlx::PgPool;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::error::{self, LoadError};
use super::trait_def::DataLoader;
use super::types::{
    BoundingBox, DataSourceMetadata, Feature, FileFormat, ImportStats, LoadedData, SourceType,
    ValidationReport,
};

/// OSM PBF data loader
///
/// Loads OpenStreetMap data from PBF files and imports into PostGIS database.
/// Follows the facade pattern by implementing the DataLoader trait.
pub struct OsmLoader {
    /// Maximum features to load in memory at once (for chunked processing)
    batch_size: usize,
}

impl OsmLoader {
    /// Create a new OSM loader with default batch size
    pub fn new() -> Self {
        Self {
            batch_size: 10_000,
        }
    }

    /// Create a new OSM loader with custom batch size
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self { batch_size }
    }

    /// Convert OSM tags to JSON
    fn tags_to_json(tags: &osmpbfreader::Tags) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (key, value) in tags.iter() {
            map.insert(key.to_string(), serde_json::Value::String(value.to_string()));
        }
        serde_json::Value::Object(map)
    }

    /// Extract primary feature type from tags (highway, building, waterway, etc.)
    fn extract_feature_type(tags: &osmpbfreader::Tags) -> Result<String, LoadError> {
        // Priority order for feature type extraction
        let priority_keys = [
            "highway", "building", "waterway", "railway", "landuse",
            "natural", "amenity", "leisure", "shop", "tourism"
        ];

        for key in &priority_keys {
            if let Some(value) = tags.get(*key) {
                return Ok(format!("{}:{}", key, value));
            }
        }

        Err(LoadError::NoFeatureType)
    }

    /// Calculate bounding box from features
    fn calculate_bbox(features: &[Feature]) -> Result<BoundingBox, LoadError> {
        if features.is_empty() {
            return Err(LoadError::EmptyFeatureSet);
        }

        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;

        for feature in features {
            match &feature.geometry {
                Geometry::Point(p) => {
                    min_x = min_x.min(p.x());
                    min_y = min_y.min(p.y());
                    max_x = max_x.max(p.x());
                    max_y = max_y.max(p.y());
                }
                Geometry::LineString(ls) => {
                    for point in ls.0.iter() {
                        min_x = min_x.min(point.x);
                        min_y = min_y.min(point.y);
                        max_x = max_x.max(point.x);
                        max_y = max_y.max(point.y);
                    }
                }
                Geometry::Polygon(poly) => {
                    for point in poly.exterior().0.iter() {
                        min_x = min_x.min(point.x);
                        min_y = min_y.min(point.y);
                        max_x = max_x.max(point.x);
                        max_y = max_y.max(point.y);
                    }
                }
                _ => {} // Handle other geometry types as needed
            }
        }

        Ok(BoundingBox::new(min_x, min_y, max_x, max_y))
    }
}

impl Default for OsmLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataLoader for OsmLoader {
    fn can_handle(&self, file_path: &Path) -> bool {
        file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pbf"))
            .unwrap_or(false)
    }

    async fn load(&self, file_path: &Path) -> Result<LoadedData, LoadError> {
        info!("Loading OSM PBF file: {}", file_path.display());

        let file = File::open(file_path).context(error::FileOpenSnafu)?;

        let _pbf = OsmPbfReader::new(file);
        let mut features = Vec::new();
        let mut nodes = HashMap::new();

        // First pass: collect all nodes
        debug!("First pass: collecting nodes");
        let mut node_count = 0;
        let mut way_count = 0;

        // We need to iterate twice, so we'll read the file twice
        let file = File::open(file_path).context(error::FileOpenSnafu)?;
        let mut pbf = OsmPbfReader::new(file);

        for obj in pbf.iter() {
            match obj.context(error::OsmParseSnafu)? {
                OsmObj::Node(node) => {
                    nodes.insert(
                        node.id,
                        Point::new(node.decimicro_lon as f64 / 1e7, node.decimicro_lat as f64 / 1e7),
                    );
                    node_count += 1;

                    // Add nodes with tags as point features
                    if !node.tags.is_empty() {
                        let mut properties = Self::tags_to_json(&node.tags);

                        // Add metadata
                        if let serde_json::Value::Object(ref mut map) = properties {
                            map.insert("osm_type".to_string(), serde_json::Value::String("node".to_string()));
                            if let Ok(ft) = Self::extract_feature_type(&node.tags) {
                                map.insert("feature_type".to_string(), serde_json::Value::String(ft));
                            }
                        }

                        features.push(Feature {
                            id: Some(node.id.0),
                            geometry: Geometry::Point(Point::new(
                                node.decimicro_lon as f64 / 1e7,
                                node.decimicro_lat as f64 / 1e7,
                            )),
                            properties,
                            feature_type: "node".to_string(),
                        });
                    }
                }
                OsmObj::Way(way) => {
                    way_count += 1;

                    // Build LineString from way nodes
                    let points: Vec<geo_types::Coord<f64>> = way
                        .nodes
                        .iter()
                        .filter_map(|node_id| nodes.get(node_id))
                        .map(|p| geo_types::Coord { x: p.x(), y: p.y() })
                        .collect();

                    if points.len() >= 2 {
                        let geometry = if points.first() == points.last() && points.len() >= 4 {
                            // Closed way -> Polygon
                            Geometry::Polygon(Polygon::new(LineString::from(points), vec![]))
                        } else {
                            // Open way -> LineString
                            Geometry::LineString(LineString::from(points))
                        };

                        let mut properties = Self::tags_to_json(&way.tags);

                        // Add metadata
                        if let serde_json::Value::Object(ref mut map) = properties {
                            map.insert("osm_type".to_string(), serde_json::Value::String("way".to_string()));
                            if let Ok(ft) = Self::extract_feature_type(&way.tags) {
                                map.insert("feature_type".to_string(), serde_json::Value::String(ft));
                            }
                        }

                        features.push(Feature {
                            id: Some(way.id.0),
                            geometry,
                            properties,
                            feature_type: "way".to_string(),
                        });
                    }
                }
                OsmObj::Relation(_) => {
                    // Relations are more complex - skip for now
                    // TODO: Implement relation handling in future
                }
            }
        }

        info!(
            "Loaded {} nodes and {} ways, created {} features",
            node_count,
            way_count,
            features.len()
        );

        let bbox = Self::calculate_bbox(&features).ok();
        let file_size = std::fs::metadata(file_path)
            .context(error::FileMetadataSnafu)?
            .len() as i64;

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(LoadError::InvalidFileName)?
            .to_string();

        let metadata = DataSourceMetadata {
            source_type: SourceType::Osm,
            file_name,
            file_format: FileFormat::Pbf,
            row_count: Some(features.len() as i64),
            file_size_bytes: Some(file_size),
            bbox,
            custom_metadata: serde_json::json!({
                "node_count": node_count,
                "way_count": way_count,
            }),
        };

        Ok(LoadedData { features, metadata })
    }

    async fn validate(&self, data: &LoadedData) -> Result<ValidationReport, LoadError> {
        let mut warnings = Vec::new();

        // Check for empty dataset
        if data.features.is_empty() {
            return Ok(ValidationReport::invalid(vec![
                "No features found in OSM file".to_string()
            ]));
        }

        // Check for missing geometries
        let empty_geom_count = data
            .features
            .iter()
            .filter(|f| match &f.geometry {
                Geometry::Point(_) => false,
                Geometry::LineString(ls) => ls.0.is_empty(),
                Geometry::Polygon(p) => p.exterior().0.is_empty(),
                _ => false,
            })
            .count();

        if empty_geom_count > 0 {
            warnings.push(format!(
                "{} features have empty geometries",
                empty_geom_count
            ));
        }

        // Check for invalid coordinates
        let invalid_coord_count = data
            .features
            .iter()
            .filter(|f| match &f.geometry {
                Geometry::Point(p) => {
                    p.x() < -180.0 || p.x() > 180.0 || p.y() < -90.0 || p.y() > 90.0
                }
                _ => false,
            })
            .count();

        if invalid_coord_count > 0 {
            warnings.push(format!(
                "{} features have invalid coordinates (outside WGS84 bounds)",
                invalid_coord_count
            ));
        }

        Ok(ValidationReport {
            is_valid: true,
            errors: Vec::new(),
            warnings,
        })
    }

    async fn metadata(&self, file_path: &Path) -> Result<DataSourceMetadata, LoadError> {
        let file_size = std::fs::metadata(file_path)
            .context(error::FileMetadataSnafu)?
            .len() as i64;

        // Quick metadata extraction without full load
        let file = File::open(file_path).context(error::FileOpenSnafu)?;
        let mut pbf = OsmPbfReader::new(file);

        let mut node_count = 0;
        let mut way_count = 0;
        let mut relation_count = 0;

        // Sample first N objects for quick metadata
        for (i, obj) in pbf.iter().enumerate() {
            if i > 1000 {
                break; // Sample only first 1000 objects
            }

            match obj.context(error::OsmParseSnafu)? {
                OsmObj::Node(_) => node_count += 1,
                OsmObj::Way(_) => way_count += 1,
                OsmObj::Relation(_) => relation_count += 1,
            }
        }

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(LoadError::InvalidFileName)?
            .to_string();

        Ok(DataSourceMetadata {
            source_type: SourceType::Osm,
            file_name,
            file_format: FileFormat::Pbf,
            row_count: None, // Would need full scan
            file_size_bytes: Some(file_size),
            bbox: None, // Would need full scan
            custom_metadata: serde_json::json!({
                "sample_node_count": node_count,
                "sample_way_count": way_count,
                "sample_relation_count": relation_count,
                "note": "Counts are from first 1000 objects only"
            }),
        })
    }

    async fn import_to_db(
        &self,
        pool: &PgPool,
        data: LoadedData,
        source_id: Uuid,
    ) -> Result<ImportStats, LoadError> {
        let start = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut rows_inserted = 0i64;

        info!("Importing {} features to database", data.features.len());

        // Process features in batches
        for chunk in data.features.chunks(self.batch_size) {
            for feature in chunk {
                // Convert geometry to GeoJSON for PostGIS
                let geom_json = match serde_json::to_string(&feature.geometry)
                    .context(error::GeometrySerializationSnafu)
                {
                    Ok(json) => json,
                    Err(e) => {
                        warn!("Skipping feature {:?}: {}", feature.id, e);
                        errors.push(format!("feature {:?}: geometry serialization failed", feature.id));
                        continue;
                    }
                };

                // Extract osm_type and feature_type from properties
                let osm_type = feature.properties
                    .get("osm_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("node");

                let feature_type_opt = feature.properties
                    .get("feature_type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Insert into osm_features table
                let result = sqlx::query(
                    r#"
                    INSERT INTO osm_features (osm_id, osm_type, feature_type, geom, tags, source_id)
                    VALUES ($1, $2, $3, ST_GeomFromGeoJSON($4), $5, $6)
                    "#
                )
                .bind(feature.id.unwrap_or(0))
                .bind(osm_type)
                .bind(feature_type_opt)
                .bind(geom_json)
                .bind(&feature.properties)
                .bind(source_id)
                .execute(pool)
                .await;

                match result.context(error::DatabaseQuerySnafu) {
                    Ok(_) => rows_inserted += 1,
                    Err(e) => {
                        warn!("Failed to insert feature {:?}: {}", feature.id, e);
                        errors.push(format!("feature {:?}: database insert failed", feature.id));
                    }
                }
            }

            debug!("Inserted {} features so far", rows_inserted);
        }

        let duration = start.elapsed();
        info!(
            "Import completed: {} rows in {:.2}s ({} errors)",
            rows_inserted,
            duration.as_secs_f64(),
            errors.len()
        );

        Ok(ImportStats {
            rows_inserted,
            duration_seconds: duration.as_secs_f64(),
            errors,
        })
    }

    fn name(&self) -> &str {
        "OsmLoader"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_handle() {
        let loader = OsmLoader::new();
        assert!(loader.can_handle(Path::new("test.pbf")));
        assert!(loader.can_handle(Path::new("test.PBF")));
        assert!(!loader.can_handle(Path::new("test.shp")));
        assert!(!loader.can_handle(Path::new("test.json")));
    }

    #[test]
    fn test_tags_to_json() {
        let mut tags = osmpbfreader::Tags::new();
        tags.insert("highway".to_string(), "residential".to_string());
        tags.insert("name".to_string(), "Main Street".to_string());

        let json = OsmLoader::tags_to_json(&tags);
        assert_eq!(json["highway"], "residential");
        assert_eq!(json["name"], "Main Street");
    }
}
