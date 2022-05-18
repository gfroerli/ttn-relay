use std::{collections::HashMap, path::PathBuf, thread, time::Duration};

use anyhow::{bail, Context, Result};
use clap::Parser;
use drogue_ttn::v3 as ttn;
use env_logger::Env;
use log::{debug, error, info, warn};
use paho_mqtt as mqtt;
use serde_json as json;

mod config;
mod influxdb;
mod payload;

use config::{Config, Sensor, SensorType};

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file
    #[clap(short, long, default_value = "config.toml")]
    config: PathBuf,
}

struct App {
    /// App configuration
    config: Config,
    /// MQTT client
    mqtt_client: mqtt::Client,
    /// HTTP client
    http_client: ureq::Agent,
}

impl App {
    fn new(config: Config) -> Result<Self> {
        // MQTT client
        let mqtt_client = mqtt::Client::new(
            mqtt::CreateOptionsBuilder::new()
                .server_uri(&config.ttn.host)
                .finalize(),
        )
        .context("Error creating the client")?;

        // HTTP client
        let http_client = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(5))
            .timeout_write(Duration::from_secs(5))
            .build();

        Ok(Self {
            config,
            mqtt_client,
            http_client,
        })
    }

    fn run(self) -> Result<()> {
        // Initialize the consumer before connecting
        let rx = self.mqtt_client.start_consuming();

        // Connect via MQTT
        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .keep_alive_interval(Duration::from_secs(20))
            .clean_session(false)
            .user_name(&self.config.ttn.user)
            .password(&self.config.ttn.pass)
            .finalize();
        let subscriptions = ["v3/+/devices/+/activations", "v3/+/devices/+/up"];
        let qos = [1, 1];
        info!("Connecting to the TTN MQTT broker...");
        let rsp = self
            .mqtt_client
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

                let qosv = self
                    .mqtt_client
                    .subscribe_many(&subscriptions, &qos)
                    .map_err(|e| {
                        self.mqtt_client.disconnect(None).unwrap();
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
                if let Err(e) = self.handle_uplink(msg) {
                    error!("Failed to handle uplink: {}", e);
                }
            } else if self.mqtt_client.is_connected() || !try_reconnect(&self.mqtt_client) {
                break;
            }
        }

        // If we're still connected, then disconnect now, otherwise we're already disconnected.
        if self.mqtt_client.is_connected() {
            info!("Disconnecting");
            self.mqtt_client.unsubscribe_many(&subscriptions).unwrap();
            self.mqtt_client.disconnect(None).unwrap();
        }
        info!("Exiting");

        Ok(())
    }

    /// Handle an uplink:
    ///
    /// - Log metadata
    /// - Look up sensor
    /// - If sensor was found, create a `MeasurementMessage` and call processing function
    fn handle_uplink(&self, msg: mqtt::Message) -> Result<()> {
        info!("Uplink received:");
        debug!("  Topic: {}", msg.topic());
        let ttn_msg: ttn::Message =
            json::from_slice(msg.payload()).context("Could not deserialize uplink payload")?;
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
        let sensor = match self.config.sensors.get(&dev_eui) {
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
        if let Err(e) = self.process_measurement(measurement_message) {
            error!("Error while processing measurement: {}", e);
        }

        Ok(())
    }

    /// Process a measurement targeted at a specific sensor.
    fn process_measurement(&self, measurement_message: MeasurementMessage) -> Result<()> {
        println!("{:?}", measurement_message);

        // Parse payload
        let parsed_data = match measurement_message.sensor.sensor_type {
            SensorType::Gfroerli => unimplemented!(),
            SensorType::Dragino => payload::parse_payload_dragino(measurement_message.raw_payload)
                .context("Failed to parse Dragino payload")?,
        };
        println!("{:?}", parsed_data);

        // Send to Gfrörli API
        if let Err(e) = self.send_to_api(
            measurement_message.sensor.sensor_id,
            parsed_data.temperature,
        ) {
            warn!("Could not submit measurement to API: {:#}", e);
        }

        // Send to InfluxDB
        if let Err(e) = self.send_to_influxdb(&measurement_message, &parsed_data) {
            warn!("Could not submit measurement to InfluxDB: {:#}", e);
        }

        info!("Processing done!");
        Ok(())
    }

    /// Send a measurement to the Gfrörli API server.
    fn send_to_api(&self, sensor_id: u32, temperature: f32) -> Result<()> {
        let url = format!("{}/measurements", self.config.api.base_url);
        let authorization = format!("Bearer {}", self.config.api.api_token);
        info!("Sending temperature {:.2}°C to API...", temperature);
        let response = self
            .http_client
            .post(&url)
            .set("authorization", &authorization)
            .send_json(&ApiPayload {
                sensor_id,
                temperature,
            })
            .context("API request failed")?;
        if response.status() == 201 {
            debug!("API request succeeded");
            Ok(())
        } else {
            bail!(
                "API request failed: HTTP {} ({})",
                response.status(),
                response.status_text()
            );
        }
    }

    /// Send a measurement to InfluxDB.
    fn send_to_influxdb(
        &self,
        measurement_message: &MeasurementMessage,
        measurement: &payload::Measurement,
    ) -> Result<()> {
        if let Some(influxdb_config) = &self.config.influxdb {
            info!("Logging measurement to InfluxDB...");

            let mut tags = HashMap::new();
            tags.insert(
                "sensor_id",
                measurement_message.sensor.sensor_id.to_string(),
            );
            tags.insert("dev_eui", measurement_message.dev_eui.to_string());
            tags.insert(
                "sensor_type",
                measurement_message.sensor.sensor_type.to_string(),
            );
            // TODO: sf, bw, best_gateway, manufacturer, protocol_version

            let mut fields = HashMap::new();
            fields.insert("water_temp", format!("{:.2}", measurement.temperature));
            fields.insert(
                "voltage",
                format!("{:.3}", (measurement.battery_millivolts as f32) / 1000.0),
            );
            // TODO: max_rssi, max_snr, enclosure_temp, enclosure_humi

            influxdb::submit_measurement(self.http_client.clone(), influxdb_config, &tags, &fields)
                .context("InfluxDB request failed")?;
            debug!("InfluxDB request succeeded");
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn,ttn_relay=debug")).init();

    info!("🥶 Gfrörli TTN Relay v3 🥶");

    // Parse args
    let cli = Cli::parse();

    // Read config
    debug!("Reading config from {:?}", &cli.config);
    let config = Config::from_file(&cli.config)?;
    info!("Configured sensors:");
    for (dev_eui, sensor) in &config.sensors {
        info!(
            "  {} → {} ({:?})",
            dev_eui, sensor.sensor_id, sensor.sensor_type
        )
    }

    // Instantiate App
    let app = App::new(config)?;
    app.run()
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

#[derive(serde::Serialize)]
struct ApiPayload {
    sensor_id: u32,
    temperature: f32,
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
