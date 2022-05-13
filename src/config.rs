use std::{collections::HashMap, fs::File, io::Read, path::Path};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// MQTT config
    pub ttn: Mqtt,
    /// A mapping from DevEUI to sensor config
    pub sensors: HashMap<String, Sensor>,
}

#[derive(Debug, Deserialize)]
pub struct Mqtt {
    /// TTN MQTT hostname
    pub host: String,
    /// Username
    pub user: String,
    /// Password
    pub pass: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum SensorType {
    /// Custom Gfrörli firmware
    Gfroerli,
    /// Dragino LSN50 v2-D20
    Dragino,
}

#[derive(Debug, Deserialize)]
pub struct Sensor {
    /// The sensor type
    pub sensor_type: SensorType,
    /// The Gfrörli API sensor ID
    pub sensor_id: u32,
}

impl Config {
    pub fn from_file(config_path: &Path) -> Result<Self> {
        // Read config file
        if !config_path.exists() {
            bail!("Config file at {:?} does not exist", config_path);
        }
        if !config_path.is_file() {
            bail!("Config file at {:?} is not a file", config_path);
        }
        let mut file = File::open(config_path).context("Could not open config file")?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .context("Could not read config file")?;

        // Deserialize
        toml::from_str(&contents).context("Could not deserialize config file")
    }
}
