use crate::config::Config;
use anyhow::Result;
use std::fmt::Display;

use time::OffsetDateTime;

pub trait Backend {
    fn get(&self, key: &str) -> Result<Vec<u8>>;
    fn delete(&self, key: &str) -> Result<()>;
    fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    fn list(&self) -> Result<Vec<File>>;
    fn new(config: &Config) -> Result<Self>
    where
        Self: Sized;
}

pub struct File {
    pub key: String,
    pub last_modified: OffsetDateTime,
}

impl Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
}
