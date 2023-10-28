use anyhow::{bail, Context, Result};
use log::{debug};
use serde::{self, Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    pub root_dir: Option<String>,
    pub remote: String,
    pub remote_profile: Option<String>,
    pub remote_region: Option<String>,
    pub remote_endpoint: Option<String>,
    pub ignore: Vec<String>,
}

impl Config {
    pub fn load(config_file_path: &Path) -> Result<Config> {
        if config_file_path.exists() {
            debug!("Loading config from {:?}", config_file_path);
            let config_data = File::open(config_file_path)
                .and_then(|mut file| {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map(|_| content)
                })
                .context("Error opening the configuration file")?;
            Ok(toml::from_str(&config_data)?)
        } else {
            bail!("Could not find the configuration file. You can set its location with --config-file or create it with the configure' command")
        }
    }

    pub fn save(&self, config_file_path: &Path) -> Result<()> {
        let mut file = File::create(config_file_path.clone())?;
        let default_content = toml::to_string(&self)?;
        file.write_all(default_content.as_bytes())?;
        Ok(())
    }
}
