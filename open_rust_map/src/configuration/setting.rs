use std::convert::TryInto;

use snafu::prelude::*;

use base::{
    configuration::environment::Environment, error, model::config_model::CliCommand, utils,
};

#[derive(serde::Deserialize, Clone)]
pub struct Settings {
    pub log_level: String,
    pub pbf_file: String,
}

pub async fn get_configuration(arg: &CliCommand) -> utils::Result<Settings> {
    let base_path = arg.config_file.clone();
    let environment: Environment = std::env::var("APP_ENVIRONMENT")
        .unwrap_or_else(|_| "local".into())
        .try_into()?;
    let environment_filename = format!("{}.toml", environment.as_str());
    let configuration_directory = base_path.unwrap_or(
        std::env::current_dir()
            .context(error::PathEnvSnafu)?
            .join("configuration")
            .join(environment_filename),
    );
    let settings = config::Config::builder()
        .add_source(config::File::from(configuration_directory))
        // Add in settings from environment variables (with a prefix of APP and '__' as separator)
        // E.g. `APP_APPLICATION__PORT=5001 would set `Settings.application.port`
        .add_source(
            config::Environment::with_prefix("APP")
                .prefix_separator("_")
                .separator("__"),
        )
        .set_default("pbf_file", arg.pbf_file.to_string())
        .context(error::ConfigEnvSnafu)?
        .build()
        .context(error::ConfigEnvSnafu)?;
    settings
        .try_deserialize::<Settings>()
        .context(error::ConfigEnvSnafu)
}
