use snafu::{Report, Snafu};

/// Analysis-specific errors
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AnalysisError {
    #[snafu(display("invalid scenario parameter: {message}"))]
    InvalidParameter { message: String },

    #[snafu(display("missing required input layer: {layer}"))]
    MissingInputLayer { layer: String },

    #[snafu(display("tile compute failed for tile {z}/{x}/{y}: {message}"))]
    TileCompute {
        z: u8,
        x: u32,
        y: u32,
        message: String,
    },

    #[snafu(display("boundary sync failed between adjacent tiles"))]
    BoundarySync { message: String },

    #[snafu(display("snapshot store error"))]
    SnapshotStore { source: sqlx::Error },

    #[snafu(display("failed to serialize scenario result"))]
    ResultSerialization { source: serde_json::Error },

    #[snafu(display("simulation did not converge within {max_iterations} iterations"))]
    NonConvergent { max_iterations: u32 },
}

impl AnalysisError {
    pub fn report(&self) {
        tracing::error!("analysis error: {}", Report::from_error(self));
    }
}
