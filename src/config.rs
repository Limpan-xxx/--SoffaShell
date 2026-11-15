use serde::Deserialize;
use std::{error::Error, path::Path};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub apps: Vec<App>,
}

#[derive(Debug, Deserialize)]
pub struct App {
    pub name: String,
    pub category: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, Box<dyn Error>> {
        let source = match path {
            Some(path) => std::fs::read_to_string(path)?,
            None => include_str!("../config/apps.toml").to_owned(),
        };
        let config: Self = toml::from_str(&source)?;
        if config.apps.is_empty() {
            return Err("Configure at least one app".into());
        }
        if config
            .apps
            .iter()
            .any(|app| app.name.trim().is_empty() || app.command.trim().is_empty())
        {
            return Err("Each app needs a non-empty name and command".into());
        }
        Ok(config)
    }
}
