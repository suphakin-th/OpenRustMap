use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[clap(version)]
pub struct CliCommand {
	#[clap(short, long)]
	pub config_file: Option<PathBuf>,
}