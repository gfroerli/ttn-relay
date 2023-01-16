use anyhow::{bail, Result};

#[derive(Debug)]
pub struct Measurement {
    /// The water temperature in °C.
    pub temperature_water: f32,
    /// The enclosure temperature in °C.
    pub temperature_enclosure: Option<f32>,
    /// The enclosure humidity in %RH.
    pub humidity_enclosure: Option<f32>,
    /// The battery voltage in millivolts.
    pub battery_millivolts: u16,
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
pub fn parse_payload_dragino(payload: &[u8]) -> Result<Measurement> {
    if payload.len() != 11 {
        bail!(
            "Expected Dragino uplink payload to be 11 bytes, but was {}",
            payload.len()
        );
    }
    let battery_millivolts = u16::from_be_bytes([payload[0], payload[1]]);
    let temperature_raw = u16::from_be_bytes([payload[2], payload[3]]) as f32;
    let temperature = match payload[2] & 0xfc == 0 {
        true => temperature_raw / 10.0,
        false => (temperature_raw - 65536.0) / 10.0,
    };
    Ok(Measurement {
        temperature_water: temperature,
        temperature_enclosure: None,
        humidity_enclosure: None,
        battery_millivolts,
    })
}

/// Parse a Gfroerli V1 payload.
///
/// Payload format: Four little endian 32-bit floats:
///
/// 1. T_water
/// 2. T_inside
/// 3. RH_inside
/// 4. V_supply
///
pub fn parse_payload_gfroerli_v1(payload: &[u8]) -> Result<Measurement> {
    if payload.len() != 16 {
        bail!(
            "Expected Gfrörli V1 uplink payload to be 16 bytes, but was {}",
            payload.len()
        );
    }
    let temperature_water = f32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let temperature_enclosure = Some(f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]));
    let humidity_enclosure = Some(f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]));
    let battery_voltage = f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
    let battery_millivolts = (battery_voltage * 1000.0) as u16;
    Ok(Measurement {
        temperature_water,
        temperature_enclosure,
        humidity_enclosure,
        battery_millivolts,
    })
}

/// Parse a Gfroerli V2 payload.
pub fn parse_payload_gfroerli_v2(_payload: &[u8]) -> Result<Measurement> {
    bail!("Gfroerli v2 support not yet implemented"); // TODO
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
        assert_eq!(measurement1.temperature_water, 26.1);
        assert_eq!(measurement2.temperature_water, -19.3);
    }

    #[test]
    fn test_parse_gfroerli_v1_payload() {
        // Payload 1: list(iter(struct.pack('<ffff', 13.14, 8.76, 75.1, 3.21)))
        let payload1 = [113, 61, 82, 65, 246, 40, 12, 65, 51, 51, 150, 66, 164, 112, 77, 64];
        // Payload 2: list(iter(struct.pack('<ffff', 20.0, 10.0, 50.5, 3.10)))
        let payload2 = [0, 0, 160, 65, 0, 0, 32, 65, 0, 0, 74, 66, 102, 102, 70, 64];
        let measurement1 = parse_payload_gfroerli_v1(&payload1).unwrap();
        let measurement2 = parse_payload_gfroerli_v1(&payload2).unwrap();
        assert_eq!(measurement1.temperature_water, 13.14);
        assert_eq!(measurement2.temperature_water, 20.0);
        assert_eq!(measurement1.temperature_enclosure, Some(8.76));
        assert_eq!(measurement2.temperature_enclosure, Some(10.0));
        assert_eq!(measurement1.humidity_enclosure, Some(75.1));
        assert_eq!(measurement2.humidity_enclosure, Some(50.5));
        assert_eq!(measurement1.battery_millivolts, 3210);
        assert_eq!(measurement2.battery_millivolts, 3100);
    }
}
