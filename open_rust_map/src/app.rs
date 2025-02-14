use time::OffsetDateTime;
use tracing;

use base::model::osm_model;
use base::utils::{self, time_diff_trace};

use crate::configuration::setting::Settings;

pub async fn run(setting: &Settings) -> utils::Result<()> {
    let _t1 = OffsetDateTime::now_utc();
    tracing::info!("Start reading pbf file: {}", &setting.pbf_file);
    let pbf = utils::read_pbf_file(&setting.pbf_file);
    tracing::info!("Start converting pbf file to osm data");
    let t2 = OffsetDateTime::now_utc();
    let osm_data = osm_model::Osm::from_osm_pbf_file(pbf);
    print!("Start update data to {:?}", osm_data);
    let t3 = OffsetDateTime::now_utc();
    time_diff_trace("Finished update data to meilisearch", t2, t3);
    Ok(())
}
