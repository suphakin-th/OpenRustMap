use geo_types::Geometry;
use serde::{Deserialize, Serialize};

/// Types of data sources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Osm,
    Gisda,
    Dem,
    Lidar,
    Custom(String),
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Osm => write!(f, "osm"),
            SourceType::Gisda => write!(f, "gisda"),
            SourceType::Dem => write!(f, "dem"),
            SourceType::Lidar => write!(f, "lidar"),
            SourceType::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// File formats supported
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Pbf,
    Shapefile,
    GeoTiff,
    Las,
    Laz,
    GeoJson,
    Gml,
    Kml,
    Custom(String),
}

impl std::fmt::Display for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileFormat::Pbf => write!(f, "pbf"),
            FileFormat::Shapefile => write!(f, "shp"),
            FileFormat::GeoTiff => write!(f, "geotiff"),
            FileFormat::Las => write!(f, "las"),
            FileFormat::Laz => write!(f, "laz"),
            FileFormat::GeoJson => write!(f, "geojson"),
            FileFormat::Gml => write!(f, "gml"),
            FileFormat::Kml => write!(f, "kml"),
            FileFormat::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// Bounding box for spatial extents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl BoundingBox {
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self {
            west,
            south,
            east,
            north,
        }
    }

    pub fn center(&self) -> (f64, f64) {
        (
            (self.west + self.east) / 2.0,
            (self.south + self.north) / 2.0,
        )
    }
}

/// Metadata about a data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceMetadata {
    pub source_type: SourceType,
    pub file_name: String,
    pub file_format: FileFormat,
    pub row_count: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub bbox: Option<BoundingBox>,
    pub custom_metadata: serde_json::Value,
}

/// A single geospatial feature
#[derive(Debug, Clone)]
pub struct Feature {
    pub id: Option<i64>,
    pub geometry: Geometry<f64>,
    pub properties: serde_json::Value,
    pub feature_type: String,
}

/// Data loaded from a file
#[derive(Debug)]
pub struct LoadedData {
    pub features: Vec<Feature>,
    pub metadata: DataSourceMetadata,
}

/// Statistics from import operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStats {
    pub rows_inserted: i64,
    pub duration_seconds: f64,
    pub errors: Vec<String>,
}

/// Validation report
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn invalid(errors: Vec<String>) -> Self {
        Self {
            is_valid: false,
            errors,
            warnings: Vec::new(),
        }
    }
}
