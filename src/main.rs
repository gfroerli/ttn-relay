use std::{collections::HashMap, path::PathBuf, time::Duration};

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

/// Main application object.
struct App {
    /// App configuration
    config: Config,
    /// MQTT client
    mqtt_client: mqtt::Client,
    /// HTTP client
    http_client: ureq::Agent,
}

#[derive(Debug)]
struct MeasurementMessage<'a> {
    dev_eui: &'a str,
    sensor: &'a Sensor,
    meta: MeasurementMeta,
    frame_port: u16,
    raw_payload: &'a [u8],
}

#[derive(Debug)]
struct MeasurementMeta {
    airtime_ms: u32,
    spreading_factor: Option<u16>,
    bandwidth: Option<u64>,
    receiving_gateways: Vec<ReceivingGateway>,
}

#[derive(Debug)]
struct ReceivingGateway {
    rssi: f64,
    snr: Option<f64>,
}

#[derive(serde::Serialize)]
struct ApiPayload {
    sensor_id: u32,
    temperature: f32,
}

static SUBSCRIPTIONS: [&str; 2] = ["v3/+/devices/+/activations", "v3/+/devices/+/up"];

impl App {
    fn new(config: Config) -> Result<Self> {
        // MQTT client
        let mut mqtt_client = mqtt::Client::new(
            mqtt::CreateOptionsBuilder::new()
                .server_uri(&config.ttn.host)
                .finalize(),
        )
        .context("Error creating the client")?;
        mqtt_client.set_timeout(Duration::from_secs(3));

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
                subscribe(&self.mqtt_client)?;
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
            } else {
                // We lost the connection. Terminate and let the relay be
                // restarted by the process manager.
                break;
            }
        }

        // If we're still connected, then disconnect now, otherwise we're already disconnected.
        if self.mqtt_client.is_connected() {
            info!("Disconnecting");
            self.mqtt_client.unsubscribe_many(&SUBSCRIPTIONS).unwrap();
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
        // Right now we're only interested in uplinks
        if !msg.topic().ends_with("/up") {
            debug!("Received a non-uplink message, ignoring");
            return Ok(());
        }
        info!("Uplink received:");
        debug!("  Topic: {}", msg.topic());

        // Decode payload and print some information
        let ttn_msg = match json::from_slice::<ttn::Message>(msg.payload()) {
            Ok(msg) => msg,
            Err(_) => {
                debug!(
                    "Uplink message could not be parsed: {}",
                    std::str::from_utf8(msg.payload())
                        .map(str::to_string)
                        .unwrap_or_else(|_| format!("{:?}", msg.payload())),
                );
                bail!("Could not deserialize uplink payload");
            }
        };
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
        let (spreading_factor, bandwidth) =
            if let Some(ttn::DataRate::Lora(dr)) = uplink.settings.data_rate {
                debug!("  SF: {}", dr.spreading_factor);
                debug!("  Bandwidth: {} Hz", dr.bandwidth);
                (Some(dr.spreading_factor), Some(dr.bandwidth))
            } else {
                warn!("Non-LoRa data rate");
                (None, None)
            };
        debug!("  Payload: {:?}", uplink.frame_payload);
        debug!("  Receiving gateways: {}", uplink.rx_metadata.len());
        let mut gateways = Vec::with_capacity(uplink.rx_metadata.len());
        for (i, gateway) in uplink.rx_metadata.iter().enumerate() {
            let name = gateway
                .gateway_ids
                .get("gateway_id")
                .or_else(|| gateway.gateway_ids.get("eui"))
                .map(String::to_string)
                .unwrap_or_else(|| format!("gateway-{}", i + 1));
            debug!("    {}: {}", i + 1, name);
            debug!(
                "       RSSI: {} / Channel RSSI: {}",
                gateway.rssi, gateway.channel_rssi
            );
            if let Some(snr) = gateway.snr {
                debug!("       SNR: {}", snr);
            } else {
                debug!("       SNR: ?");
            }
            gateways.push(ReceivingGateway {
                rssi: gateway.rssi,
                snr: gateway.snr,
            });
        }

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
                airtime_ms: uplink.consumed_airtime.num_milliseconds() as u32,
                spreading_factor,
                bandwidth,
                receiving_gateways: gateways,
            },
            frame_port: uplink.frame_port,
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
        // Parse payload
        let parsed_data = match measurement_message.sensor.sensor_type {
            // Gfroerli
            SensorType::Gfroerli if measurement_message.frame_port == 1 => {
                payload::parse_payload_gfroerli_v1(measurement_message.raw_payload)
                    .context("Failed to parse Gfroerli V1 payload")?
            }
            SensorType::Gfroerli if measurement_message.frame_port == 2 => {
                payload::parse_payload_gfroerli_v2(measurement_message.raw_payload)
                    .context("Failed to parse Gfroerli V2 payload")?
            }
            SensorType::Gfroerli => bail!(
                "Unknown FPort for a Gfroerli sensor: {}",
                measurement_message.frame_port
            ),

            // Dragino
            SensorType::Dragino => payload::parse_payload_dragino(measurement_message.raw_payload)
                .context("Failed to parse Dragino payload")?,
        };
        info!("Measurement: {:?}", parsed_data);

        // Send to Gfrörli API
        if measurement_message.sensor.send_to_api.unwrap_or(true) {
            if let Err(e) = self.send_to_api(
                measurement_message.sensor.sensor_id,
                parsed_data.temperature_water,
            ) {
                warn!("Could not submit measurement to API: {:#}", e);
            }
        } else {
            info!(
                "API data submission was disabled for sensor {}",
                measurement_message.sensor.sensor_id
            );
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

            // Tags (can be used for filtering and grouping)
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
            if let Some(sf) = measurement_message.meta.spreading_factor {
                tags.insert("sf", sf.to_string());
            }
            if let Some(bw) = measurement_message.meta.bandwidth {
                tags.insert("bw", bw.to_string());
            }

            // Value fields
            let mut fields = HashMap::new();
            fields.insert(
                "water_temp",
                format!("{:.2}", measurement.temperature_water),
            );
            if let Some(temp) = measurement.temperature_enclosure {
                fields.insert("enclosure_temp", format!("{:.2}", temp));
            }
            if let Some(humi) = measurement.humidity_enclosure {
                fields.insert("eenclosure_humi", format!("{:.2}", humi));
            }
            fields.insert(
                "voltage",
                format!("{:.3}", (measurement.battery_millivolts as f32) / 1000.0),
            );
            fields.insert(
                "airtime_ms",
                measurement_message.meta.airtime_ms.to_string(),
            );
            if let Some(sf) = measurement_message.meta.spreading_factor {
                fields.insert("sf", sf.to_string());
            }
            fields.insert(
                "receiving_gateway_count",
                measurement_message
                    .meta
                    .receiving_gateways
                    .len()
                    .to_string(),
            );
            if !measurement_message.meta.receiving_gateways.is_empty() {
                if let Some(max_rssi) = measurement_message
                    .meta
                    .receiving_gateways
                    .iter()
                    .map(|gw| gw.rssi)
                    .max_by(|a, b| a.total_cmp(b))
                {
                    fields.insert("max_rssi", max_rssi.to_string());
                }
                if let Some(max_snr) = measurement_message
                    .meta
                    .receiving_gateways
                    .iter()
                    .filter_map(|gw| gw.snr)
                    .max_by(|a, b| a.total_cmp(b))
                {
                    fields.insert("max_snr", max_snr.to_string());
                }
            }

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

/// Subscribe to activations and uplinks.
fn subscribe(client: &mqtt::Client) -> Result<()> {
    let qos = [1, 1];

    // Register subscriptions on the server
    debug!("Subscribing to topics, with requested QoS: {:?}", qos);

    let qosv = client
        .subscribe_many(&SUBSCRIPTIONS, &qos)
        .map_err(|e| {
            client.disconnect(None).unwrap();
            e
        })
        .context("Error subscribing to topics")?;
    debug!("QoS granted: {}", qosv.reason_code());
    Ok(())
}
