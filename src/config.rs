use serde::{self, Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    fs::File,
    io::{Error as IoError, Read, Write},
    path::Path,
};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    pub remote: String,
    pub remote_profile: Option<String>,
    pub anonymous: Option<bool>,
    pub ignore: Vec<String>,
}

#[derive(Debug)]
pub struct ConfigLoadError(pub Box<dyn Error>);

impl fmt::Display for ConfigLoadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error loading configuration file")
    }
}

impl Error for ConfigLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl From<IoError> for ConfigLoadError {
    fn from(value: IoError) -> Self {
        ConfigLoadError(Box::new(value))
    }
}

impl From<toml::ser::Error> for ConfigLoadError {
    fn from(value: toml::ser::Error) -> Self {
        ConfigLoadError(Box::new(value))
    }
}

impl From<toml::de::Error> for ConfigLoadError {
    fn from(value: toml::de::Error) -> Self {
        ConfigLoadError(Box::new(value))
    }
}

impl Config {
    pub fn load(config_file_path: &Path) -> Result<Config, ConfigLoadError> {
        if config_file_path.exists() {
            println!("Loading config from {:?}", config_file_path);
            let config_data = File::open(config_file_path).and_then(|mut file| {
                let mut content = String::new();
                file.read_to_string(&mut content).map(|_| content)
            })?;
            Ok(toml::from_str(&config_data)?)
        } else {
            println!("Creating config file in {:?}", config_file_path);
            let config = Config::default();
            config.save(config_file_path)?;
            Ok(config)
        }
    }

    pub fn save(&self, config_file_path: &Path) -> Result<(), ConfigLoadError> {
        let mut file = File::create(config_file_path.clone())?;
        let default_content = toml::to_string(&self)?;
        file.write_all(default_content.as_bytes())?;
        Ok(())
    }
}
