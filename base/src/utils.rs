use crate::error::Error;
use time::OffsetDateTime;
use tracing;

pub type Result<T, E = Error> = std::result::Result<T, E>;

// create function to read pbf file
pub fn read_pbf_file(filename: &str) -> osmpbfreader::OsmPbfReader<std::fs::File> {
    let path = std::path::Path::new(filename);
    tracing::debug!("start file target : {:?}", filename);
    let r = std::fs::File::open(path).unwrap();
    osmpbfreader::OsmPbfReader::new(r)
}

pub fn time_diff_trace(text: &str, from: OffsetDateTime, to: OffsetDateTime) {
	let diff = to - from;
	tracing::info!(
		"{} in {} hours, {} minutes, and {} seconds",
		text,
		diff.whole_hours(),
		diff.whole_minutes() % 60,
		diff.whole_seconds() % 60
	);
}