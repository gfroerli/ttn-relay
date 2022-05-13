use std::{path::PathBuf, thread, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use drogue_ttn::v3 as ttn;
use env_logger::Env;
use log::{debug, error, info, warn};
use paho_mqtt as mqtt;

mod config;

use config::Config;

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
    for (deveui, sensor) in config.sensors {
        info!(
            "  {} → {} ({:?})",
            deveui, sensor.sensor_id, sensor.sensor_type
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
            if let Err(e) = handle_uplink(msg) {
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

fn handle_uplink(msg: mqtt::Message) -> Result<()> {
    info!("Uplink received:");
    debug!("  Topic: {}", msg.topic());
    let ttn_msg: ttn::Message =
        serde_json::from_slice(msg.payload()).context("Could not deserialize uplink payload")?;
    info!("  DevEUI: {:?}", ttn_msg.end_device_ids.dev_eui);
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
    debug!("  Airtime: {} ms", uplink.consumed_airtime.num_milliseconds());
    if let Some(ttn::DataRate::Lora(_dr)) = uplink.settings.data_rate {
        // TODO: https://github.com/drogue-iot/drogue-ttn/pull/2
    }
    debug!("  Payload: {:?}", uplink.frame_payload);
    Ok(())
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
