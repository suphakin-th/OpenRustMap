use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[clap(version)]
pub struct CliCommand {
    #[clap(short, long)]
    pub config_file: Option<PathBuf>,
    #[clap(short, long, default_value = "../data/thailand.pbf")]
    pub pbf_file: String,
}
