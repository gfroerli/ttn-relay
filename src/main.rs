use std::{collections::HashMap, path::PathBuf, thread, time::Duration};

use anyhow::{bail, Context, Result};
use clap::Parser;
use drogue_ttn::v3 as ttn;
use env_logger::Env;
use log::{debug, error, info, warn};
use paho_mqtt as mqtt;

mod config;

use config::{Config, Sensor, SensorType};

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file
    #[clap(short, long, default_value = "config.toml")]
    config: PathBuf,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn,ttn_relay=debug")).init();

    info!("🥶 Gfrörli TTN Relay v3 🥶");

    // Parse args
    let cli = Cli::parse();

    // Read config
    debug!("Reading config from {:?}", &cli.config);
    let config: Config = Config::from_file(&cli.config)?;
    info!("Configured sensors:");
    for (dev_eui, sensor) in &config.sensors {
        info!(
            "  {} → {} ({:?})",
            dev_eui, sensor.sensor_id, sensor.sensor_type
        )
    }

    // Create MQTT client
    let client = mqtt::Client::new(
        mqtt::CreateOptionsBuilder::new()
            .server_uri(&config.ttn.host)
            .finalize(),
    )
    .context("Error creating the client")?;

    // Initialize the consumer before connecting
    let rx = client.start_consuming();

    // Connect via MQTT
    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(false)
        .user_name(&config.ttn.user)
        .password(&config.ttn.pass)
        .finalize();
    let subscriptions = ["v3/+/devices/+/activations", "v3/+/devices/+/up"];
    let qos = [1, 1];
    info!("Connecting to the TTN MQTT broker...");
    let rsp = client
        .connect(conn_opts)
        .context("Error connecting to the broker")?;
    if let Some(conn_rsp) = rsp.connect_response() {
        debug!(
            "Connected to: '{}' with MQTT version {}",
            conn_rsp.server_uri, conn_rsp.mqtt_version
        );
        if !conn_rsp.session_present {
            // Register subscriptions on the server
            debug!("Subscribing to topics, with requested QoS: {:?}", qos);

            let qosv = client
                .subscribe_many(&subscriptions, &qos)
                .map_err(|e| {
                    client.disconnect(None).unwrap();
                    e
                })
                .context("Error subscribing to topics")?;
            debug!("QoS granted: {}", qosv.reason_code());
        }
    }

    // Just loop on incoming messages.
    // If we get a `None` message, check if we got disconnected, and then try a reconnect.
    info!("Waiting for messages...");
    for msg in rx.iter() {
        if let Some(msg) = msg {
            if let Err(e) = handle_uplink(msg, &config.sensors) {
                error!("Failed to handle uplink: {}", e);
            }
        } else if client.is_connected() || !try_reconnect(&client) {
            break;
        }
    }

    // If we're still connected, then disconnect now, otherwise we're already disconnected.
    if client.is_connected() {
        info!("Disconnecting");
        client.unsubscribe_many(&subscriptions).unwrap();
        client.disconnect(None).unwrap();
    }
    info!("Exiting");

    Ok(())
}

#[derive(Debug)]
struct MeasurementMessage<'a> {
    dev_eui: &'a str,
    sensor: &'a Sensor,
    meta: MeasurementMeta,
    raw_payload: &'a [u8],
}

#[derive(Debug)]
struct MeasurementMeta {
    frame_port: u16,
    airtime_ms: i64,
}

/// Handle an uplink:
///
/// - Log metadata
/// - Look up sensor
/// - If sensor was found, create a `MeasurementMessage` and call processing function
fn handle_uplink(msg: mqtt::Message, sensors: &HashMap<String, Sensor>) -> Result<()> {
    info!("Uplink received:");
    debug!("  Topic: {}", msg.topic());
    let ttn_msg: ttn::Message =
        serde_json::from_slice(msg.payload()).context("Could not deserialize uplink payload")?;
    let dev_eui = ttn_msg.end_device_ids.dev_eui;
    info!("  DevEUI: {:?}", dev_eui);
    debug!("  DevAddr: {:?}", ttn_msg.end_device_ids.dev_addr);
    let uplink = match ttn_msg.payload {
        ttn::Payload::JoinAccept(_) => {
            info!("  Join accept, ignoring");
            return Ok(());
        }
        ttn::Payload::Uplink(uplink) => uplink,
    };
    debug!("  FPort: {}", uplink.frame_port);
    debug!("  FCnt: {:?}", uplink.frame_counter);
    debug!(
        "  Airtime: {} ms",
        uplink.consumed_airtime.num_milliseconds()
    );
    if let Some(ttn::DataRate::Lora(_dr)) = uplink.settings.data_rate {
        // TODO: https://github.com/drogue-iot/drogue-ttn/pull/2
    }
    debug!("  Payload: {:?}", uplink.frame_payload);

    // Look up sensor
    let sensor = match sensors.get(&dev_eui) {
        Some(s) => s,
        None => {
            warn!(
                "Sensor with DevEUI {} not found in config, ignoring uplink",
                dev_eui
            );
            return Ok(());
        }
    };

    // Collect relevant information
    let measurement_message = MeasurementMessage {
        dev_eui: &dev_eui,
        sensor,
        meta: MeasurementMeta {
            frame_port: uplink.frame_port,
            airtime_ms: uplink.consumed_airtime.num_milliseconds(),
        },
        raw_payload: &uplink.frame_payload,
    };

    // Process measurement
    process_measurement(measurement_message);

    Ok(())
}

/// Process a measurement targeted at a specific sensor.
fn process_measurement(measurement_message: MeasurementMessage) {
    println!("{:?}", measurement_message);
    let parsed_data = match measurement_message.sensor.sensor_type {
        SensorType::Gfroerli => unimplemented!(),
        SensorType::Dragino => parse_payload_dragino(measurement_message.raw_payload),
    };
    println!("{:?}", parsed_data);
}

#[derive(Debug)]
struct Measurement {
    /// The battery voltage in millivolts.
    battery_millivolts: u16,
    /// The water temperature in °C.
    temperature: f32,
}

/// Parse a Dragino payload.
///
/// Payload format:
///
/// - 2 bytes battery voltage
/// - 2 bytes temperature
/// - 2 bytes reserved
/// - 1 byte alarm flag
/// - 4 bytes for other temperature sensors (unused)
///
/// All multi-byte values are in big endian format.
fn parse_payload_dragino(payload: &[u8]) -> Result<Measurement> {
    if payload.len() != 11 {
        bail!("Expected Dragino uplink payload to be 11 bytes, but was {}", payload.len());
    }
    let battery_millivolts = u16::from_be_bytes([payload[0], payload[1]]);
    let temperature_raw = u16::from_be_bytes([payload[2], payload[3]]) as f32;
    let temperature = match payload[2] & 0xfc == 0 {
        true => temperature_raw / 10.0,
        false => (temperature_raw - 65536.0) / 10.0,
    };
    Ok(Measurement {
        battery_millivolts,
        temperature,
    })
}

/// Attempt to reconnect to the broker. It can be called after connection is lost.
fn try_reconnect(client: &mqtt::Client) -> bool {
    warn!("Connection lost. Waiting to retry connection");
    loop {
        thread::sleep(Duration::from_millis(5000));
        if client.reconnect().is_ok() {
            info!("Successfully reconnected");
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dragino_payload() {
        // Test values taken from datasheet
        let payload1 = [0x0b, 0x45, 0x01, 0x05, 0, 0, 0, 0, 0, 0, 0];
        let payload2 = [0x0b, 0x49, 0xff, 0x3f, 0, 0, 0, 0, 0, 0, 0];
        let measurement1 = parse_payload_dragino(&payload1).unwrap();
        let measurement2 = parse_payload_dragino(&payload2).unwrap();
        assert_eq!(measurement1.battery_millivolts, 2885);
        assert_eq!(measurement2.battery_millivolts, 2889);
        assert_eq!(measurement1.temperature, 26.1);
        assert_eq!(measurement2.temperature, -19.3);
    }
}
