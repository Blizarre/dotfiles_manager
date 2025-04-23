use anyhow::{bail, Context, Result};
use log::debug;
use serde::{self, Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    pub url: String,
    pub root_dir: Option<String>,
    pub ignore: Vec<String>,
}

impl Config {
    pub fn load(config_file_path: &Path) -> Result<Config> {
        let mut config: Config = if config_file_path.exists() {
            debug!("Loading config from {:?}", config_file_path);
            let config_data = File::open(config_file_path)
                .and_then(|mut file| {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map(|_| content)
                })
                .context("Error opening the configuration file")?;
            toml::from_str(&config_data)?
        } else {
            Config::default()
        };

        let _ = std::env::var("DOT_URL").map(|val| config.url = val);

        if config.url == String::default() {
            bail!("Could not find the configuration file. You can set its location with --config-file or create it with the configure' command. You can also set DOT_REMOTE without a configuration file")
        }
        Ok(config)
    }

    pub fn save(&self, config_file_path: &Path) -> Result<()> {
        let mut file = File::create(config_file_path)?;
        let default_content = toml::to_string(&self)?;
        file.write_all(default_content.as_bytes())?;
        Ok(())
    }
}
