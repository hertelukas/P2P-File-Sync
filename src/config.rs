use dirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to read file {path:?}")]
    ReadError {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("No config directory found.")]
    NoConfigDir,
    #[error(transparent)]
    ParseError(#[from] toml::de::Error),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub paths: Vec<String>,
}

impl Config {
    /// Loads a configuration file from disk.
    pub fn load() -> Result<Self, Error> {
        let path = Self::config_path()?;
        let config =
            fs::read_to_string(&path).map_err(|source| Error::ReadError { path, source })?;
        toml::from_str(&config).map_err(Error::from)
    }

    fn config_path() -> Result<PathBuf, Error> {
        if let Some(config_path) = dirs::config_dir() {
            Ok(config_path.join("p2p").join("config.toml"))
        } else {
            Err(Error::NoConfigDir)
        }
    }
}
