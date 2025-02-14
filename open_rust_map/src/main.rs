use clap::Parser;
use open_rust_map::{
    app::run,
    configuration::{setting, SETTINGS},
};

use base::{model::config_model, utils};

#[tokio::main]
async fn main() -> utils::Result<()> {
    let arg = config_model::CliCommand::parse();
    let config = setting::get_configuration(&arg).await?;
    let setting = SETTINGS.get_or_init(|| async move { config }).await;
    run(setting).await
}
