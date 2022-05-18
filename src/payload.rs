use anyhow::{bail, Result};

#[derive(Debug)]
pub struct Measurement {
    /// The battery voltage in millivolts.
    pub battery_millivolts: u16,
    /// The water temperature in °C.
    pub temperature: f32,
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
        battery_millivolts,
        temperature,
    })
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
