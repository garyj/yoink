//! Settings read from the user's config file.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::DEFAULT_MAX_ITEMS;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Items kept in the history. Zero stores nothing.
    pub max_items: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_items: DEFAULT_MAX_ITEMS,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read(e) => write!(f, "cannot read config: {e}"),
            ConfigError::Parse(e) => write!(f, "invalid config: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads the TOML file at `path`. A missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Read(e)),
        }
    }

    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        toml::from_str(text).map_err(ConfigError::Parse)
    }
}
