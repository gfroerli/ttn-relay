use std::collections::HashMap;

use anyhow::{Context, Result};
use log::debug;
use ureq::Agent;

use crate::config;

pub enum InfluxDbConfig<'a> {
    V1(&'a config::InfluxDb),
    V2(&'a config::InfluxDb2),
}

pub fn submit_measurement(
    agent: Agent,
    config: InfluxDbConfig,
    tags: &HashMap<&'static str, String>,
    fields: &HashMap<&'static str, String>,
) -> Result<()> {
    // Prepare payloads
    let mut payloads = vec![];
    let tags_string = tags
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>()
        .join(",");
    let fields_string = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>()
        .join(",");
    let default_measurement = "temperature";
    let measurement = match config {
        InfluxDbConfig::V1(c) => c.measurement.as_deref().unwrap_or(default_measurement),
        InfluxDbConfig::V2(c) => c.measurement.as_deref().unwrap_or(default_measurement),
    };
    payloads.push(format!("{},{} {}", measurement, tags_string, fields_string));
    let payload = payloads.join("\n");
    debug!("Sending payload: {}", payload);

    // Create basic auth header
    let auth = match config {
        InfluxDbConfig::V1(c) => {
            format!(
                "Basic {}",
                base64::encode(format!("{}:{}", &c.user, &c.pass))
            )
        }
        InfluxDbConfig::V2(c) => {
            format!("Token {}", &c.api_token)
        }
    };

    // Create request
    let url = match config {
        InfluxDbConfig::V1(c) => format!("{}/write?db={}", c.base_url, c.db),
        InfluxDbConfig::V2(c) => format!("{}/api/v2/write?org={}&bucket={}", c.base_url, c.org, c.bucket),
    };

    // Send request to server
    agent
        .post(&url)
        .set("authorization", &auth)
        .send_string(&payload)
        .context("HTTP request failed")?;

    Ok(())
}
