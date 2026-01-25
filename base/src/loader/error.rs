use snafu::{Snafu, Report};

/// Loader-specific errors
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum LoadError {
    #[snafu(display("failed to open file"))]
    FileOpen { source: std::io::Error },

    #[snafu(display("failed to read file metadata"))]
    FileMetadata { source: std::io::Error },

    #[snafu(display("failed to parse OSM object"))]
    OsmParse { source: osmpbfreader::Error },

    #[snafu(display("validation error: {message}"))]
    Validation { message: String },

    #[snafu(display("unsupported file format: {format}"))]
    UnsupportedFormat { format: String },

    #[snafu(display("database query failed"))]
    DatabaseQuery { source: sqlx::Error },

    #[snafu(display("failed to serialize geometry"))]
    GeometrySerialization { source: serde_json::Error },

    #[snafu(display("GDAL error: {message}"))]
    Gdal { message: String },

    #[snafu(display("no features found in file"))]
    EmptyFeatureSet,

    #[snafu(display("no feature type tags found"))]
    NoFeatureType,

    #[snafu(display("invalid file name"))]
    InvalidFileName,
}

impl LoadError {
    pub fn report(&self) {
        match self {
            e @ LoadError::EmptyFeatureSet => {
                tracing::warn!("{}", Report::from_error(e));
            }
            e @ LoadError::NoFeatureType => {
                tracing::debug!("{}", Report::from_error(e));
            }
            e => {
                tracing::error!("loader error: {}", Report::from_error(e));
            }
        }
    }
}
