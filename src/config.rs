use std::{collections::HashMap, fmt, fs::File, io::Read, path::Path};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// MQTT config
    pub ttn: Mqtt,
    /// API config
    pub api: Api,
    /// InfluxDB config
    pub influxdb: Option<InfluxDb>,
    /// InfluxDB 2 config (has precedence over InfluxDB 1)
    pub influxdb2: Option<InfluxDb2>,
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
pub struct Api {
    /// Gfrörli API base URL
    pub base_url: String,
    /// API token
    pub api_token: String,
}

#[derive(Debug, Deserialize)]
pub struct InfluxDb {
    /// InfluxDB connection string, e.g. `https://influxdb.example.com`
    pub base_url: String,
    /// InfluxDB username
    pub user: String,
    /// InfluxDB password
    pub pass: String,
    /// InfluxDB database
    pub db: String,
    /// Measurement name (default: "temperature")
    pub measurement: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InfluxDb2 {
    /// InfluxDB connection string, e.g. `https://influxdb.example.com`
    pub base_url: String,
    /// InfluxDB organization (name or ID)
    pub org: String,
    /// InfluxDB API token
    pub api_token: String,
    /// InfluxDB bucket
    pub bucket: String,
    /// Measurement name (default: "temperature")
    pub measurement: Option<String>,
}

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum SensorType {
    /// Custom Gfrörli firmware
    Gfroerli,
    /// Dragino LSN50 v2-D20
    Dragino,
}

impl fmt::Display for SensorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            SensorType::Gfroerli => "gfroerli",
            SensorType::Dragino => "dragino",
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Sensor {
    /// The sensor type
    pub sensor_type: SensorType,
    /// The Gfrörli API sensor ID
    pub sensor_id: u32,
    /// Whether to send data of this sensor to the API (default true)
    ///
    /// If set to false, data will be logged to InfluxDB, but not to the
    /// Gfroerli API.
    pub send_to_api: Option<bool>,
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
