use tracing::info;
use time::OffsetDateTime;

use base::model::osm_data_model;
use base::util;

use crate::configuration::setting::Settings;
use crate::model::builder::builders::MeiliSearchMasterDataBuilder;
use crate::service::util::time_diff_trace;

pub async fn run(setting: &Settings) -> util::Result<()> {
	let t1 = OffsetDateTime::now_utc();
	info!("Start reading pbf file: {}", &setting.pbf_file);
	let pbf = util::read_pbf_file(&setting.pbf_file);
	info!("Start converting pbf file to osm data");
	let t2 = OffsetDateTime::now_utc();
	let osm_data = osm_data_model::OsmData::from_osm_pbf_file(pbf);
	let t3 = OffsetDateTime::now_utc();
	Ok(())
}
