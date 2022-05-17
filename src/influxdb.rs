use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result};
use log::debug;
use ureq::Agent;

use crate::config;

/// Create an ureq agent.
pub fn make_ureq_agent() -> Agent {
    ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build()
}

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
    payloads.push(format!("temperature_test,{} {}", tags_string, fields_string));
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
