use std::collections::HashMap;

use anyhow::{Context, Result};
use log::debug;
use ureq::Agent;

use crate::config;

pub fn submit_measurement(
    agent: Agent,
    config: &config::InfluxDb,
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
    let measurement = config.measurement.as_deref().unwrap_or("temperature");
    payloads.push(format!("{},{} {}", measurement, tags_string, fields_string));
    let payload = payloads.join("\n");
    debug!("Sending payload: {}", payload);

    // Create basic auth header
    let auth = format!(
        "Basic {}",
        base64::encode(format!("{}:{}", &config.user, &config.pass))
    );

    // Create request
    let url = format!("{}/write?db={}", config.base_url, config.db);

    // Send request to server
    agent
        .post(&url)
        .set("authorization", &auth)
        .send_string(&payload)
        .context("HTTP request failed")?;

    Ok(())
}
