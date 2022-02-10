"""
Water Sensor Project: Relay from TTN to API.
"""
from pprint import pprint
import base64
from dataclasses import dataclass
import datetime
import json
import os
import ssl
import struct
import sys
import time
from typing import Any, Dict, Optional

import bitstruct
import dotenv
import influxdb
import paho.mqtt.client as mqtt
import requests


# Helper function
def require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError('Missing {} env variable'.format(name))
    return value.strip()


# Config from .env file
dotenv.load_dotenv(dotenv.find_dotenv())

# General
DEBUG = os.environ.get('DEBUG', '0').lower() in ['1', 'true', 'yes', 'y']

# TTN
TTN_MQTT_ENDPOINT = os.environ.get('TTN_MQTT_ENDPOINT', 'eu1.cloud.thethings.network')
TTN_MQTT_USERNAME = require_env('TTN_MQTT_USERNAME')
TTN_MQTT_PASSWORD = require_env('TTN_MQTT_PASSWORD')

# InfluxDB
INFLUXDB_HOST = require_env('INFLUXDB_HOST')
INFLUXDB_PORT = require_env('INFLUXDB_PORT')
INFLUXDB_USER = require_env('INFLUXDB_USER')
INFLUXDB_PASS = require_env('INFLUXDB_PASS')
INFLUXDB_DB = require_env('INFLUXDB_DB')

# Watertemp API
API_URL = 'https://watertemp-api.coredump.ch/api'
API_TOKEN = os.environ.get('API_TOKEN')

# Return code mapping
CONNECT_RETURN_CODES = {
    0: 'Connection Accepted',
    1: 'Connection Refused, unacceptable protocol version',
    2: 'Connection Refused, identifier rejected',
    3: 'Connection Refused, Server unavailable',
    4: 'Connection Refused, bad user name or password',
    5: 'Connection Refused, not authorized',
}

# Sensor mappings
SENSOR_MAPPINGS_RAW = os.environ.get('SENSOR_MAPPINGS', '')
if len(SENSOR_MAPPINGS_RAW) == 0:
    print('Missing SENSOR_MAPPINGS env var')
    sys.exit(1)
tmp = SENSOR_MAPPINGS_RAW.split(',')
SENSOR_MAPPINGS = dict(zip(tmp[::2], map(int, tmp[1::2])))

# FPorts
FPORT_FORMAT_V1 = 1
FPORT_FORMAT_V2 = 2
SUPPORTED_FPORTS = [
    FPORT_FORMAT_V1,
    FPORT_FORMAT_V2,
]

# Create InfluxDB client
influxdb_client = influxdb.InfluxDBClient(
    INFLUXDB_HOST, INFLUXDB_PORT,
    INFLUXDB_USER, INFLUXDB_PASS,
    INFLUXDB_DB,
    ssl=True, verify_ssl=True, timeout=10,
)


@dataclass
class Measurements:
    t_water: Optional[float] = None
    t_inside: Optional[float] = None
    rh_inside: Optional[float] = None
    v_supply: Optional[float] = None


def send_to_api(
    sensor_id: int,
    temperature: float,
    attributes: dict,
):
    """
    Send temperature measurement to API.

    TODO: Send along attributes (if defined)
    """
    data = {
        'sensor_id': sensor_id,
        'temperature': temperature,
    }
    headers = {
        'Authorization': 'Bearer %s' % API_TOKEN,
    }
    print('Sending temperature %.2f°C to API...' % temperature, end='')
    resp = requests.post(API_URL + '/measurements', json=data, headers=headers)
    if resp.status_code == 201:
        print(' OK')
    else:
        print(' HTTP%d' % resp.status_code)


def log_to_influxdb(
    client: influxdb.InfluxDBClient,
    fields: Dict[str, float],
    tags: Dict[str, Any],
):
    """
    Log the specified data to InfluxDB.
    """
    json_body = [{
        'measurement': 'temperature',
        'tags': tags,
        'fields': fields,
    }]
    client.write_points(json_body)


def on_connect(client, userdata, flags, rc):
    print('Connected with result code %s: %s' % (rc, CONNECT_RETURN_CODES.get(rc, 'Unknown')))
    if rc == 0:
        # Subscribe to all topics
        print('Subscribing to all topics ("#")')
        client.subscribe('#')


def on_message(client, userdata, msg):
    print('\n%s' % msg.topic)
    data = json.loads(msg.payload.decode('utf8'))
    if DEBUG:
        pprint(data)
    handle_message(msg.topic, data)


def handle_message(topic: str, data: dict):
    # Get general information
    device_id = data['end_device_ids']['device_id']
    dev_eui = data['end_device_ids']['dev_eui']
    dev_addr = data['end_device_ids']['dev_addr']
    application_id = data['end_device_ids']['application_ids']['application_id']
    print('General information:')
    print('  Application ID: %s' % application_id)
    print('  Device ID: %s' % device_id)
    print('  Dev EUI: %s' % dev_eui)
    print('  Dev address: %s' % dev_addr)

    # Right now we're only interested in uplinks
    if not topic.endswith('/up'):
        print('Not an uplink, ignoring')
        return

    # Get uplink message
    uplink = data['uplink_message']

    # Print some metadata
    print('Uplink metadata:')
    print('  FPort: %s' % uplink['f_port'])
    spreading_factor = int(uplink['settings']['data_rate']['lora']['spreading_factor'])
    print('  SF: %s' % spreading_factor)
    bandwidth_khz = int(uplink['settings']['data_rate']['lora']['bandwidth'] / 1000)
    print('  BW: %s KHz' % bandwidth_khz)
    print('  Frequency: %.2f MHz' % (int(uplink['settings']['frequency']) / 1000 / 1000))
    print('  Airtime: %s' % uplink['consumed_airtime'])

    # Receiving gateways
    print('Receiving gateways:')
    for gw in uplink['rx_metadata']:
        print('  - ID: %s (EUI %s)' % (
            gw['gateway_ids']['gateway_id'],
            gw['gateway_ids'].get('eui', '?'),
        ))
        print('    RSSI: %s' % gw.get('rssi', '-'))
        print('    SNR: %s' % gw.get('snr', '-'))

    # Get max RSSI/SNR values
    gateways = [
        g for g in uplink['rx_metadata']
        if g.get('rssi') is not None and g.get('snr') is not None
    ]
    all_rssi = [g['rssi'] for g in gateways]
    all_snr = [g['snr'] for g in gateways]
    max_rssi = max(all_rssi) if len(all_rssi) > 0 else None
    max_snr = max(all_snr) if len(all_snr) > 0 else None

    print('  Summary:')
    print('    Max RSSI: %s' % max_rssi)
    print('    Max SNR: %s' % max_snr)

    # Get gateway with best reception
    best_gateway = sorted(gateways, key=lambda g: g['rssi'], reverse=True)[0]
    best_gateway_id = best_gateway['gateway_ids']['gateway_id']
    best_gateway_eui = best_gateway['gateway_ids'].get('eui', '?')
    print('    Best gateway (by RSSI): %s (EUI %s)' % (best_gateway_id, best_gateway_eui))

    # Print payload
    print('Payload:')
    print('  Raw payload: %s' % uplink['frm_payload'])

    # Filter by FPort
    fport = uplink['f_port']
    if fport not in SUPPORTED_FPORTS:  # Hardware v1
        print('Not an FPort we can handle, ignoring')
        return

    # Prepare data
    measurements = Measurements()

    # Decode raw payload
    bytestring = base64.b64decode(uplink['frm_payload'])

    if fport == FPORT_FORMAT_V1:
        protocol_version = 'v1'

        # Decode message bytes
        PAYLOAD_FORMAT_V1 = '<ffff'
        try:
            unpacked = struct.unpack(PAYLOAD_FORMAT_V1, bytestring)
        except struct.error as e:
            print('Invalid payload v1 format: %s' % e)
            return
        measurements.t_water = unpacked[0]
        measurements.t_inside = unpacked[1]
        measurements.rh_inside = unpacked[2]
        measurements.v_supply = unpacked[3]

    elif fport == FPORT_FORMAT_V2:
        protocol_version = 'v2'

        # Decode message bytes
        data_mask = bytestring[0]
        data = bytestring[1:]

        # Data mask bits
        DM_BIT_T_WATER = 1 << 0
        DM_BIT_T_INSIDE = 1 << 1
        DM_BIT_RH_INSIDE = 1 << 2
        DM_BIT_V_SUPPLY = 1 << 3

        # Determine bitstruct format
        bitstruct_fmt = '>'
        if data_mask & DM_BIT_T_WATER:
            # Water temperature
            bitstruct_fmt += 'u12'
        if data_mask & DM_BIT_T_INSIDE:
            # Inside temperature
            bitstruct_fmt += 'u16'
        if data_mask & DM_BIT_RH_INSIDE:
            # Inside humidity
            bitstruct_fmt += 'u16'
        if data_mask & DM_BIT_V_SUPPLY:
            # Supply voltage
            bitstruct_fmt += 'u12'

        # Decode bitstruct
        try:
            raw_values = bitstruct.unpack(bitstruct_fmt, data)
        except bitstruct.Error as e:
            print('Invalid payload v2 format: %s' % e)
            return

        # Convert measurements
        index = 0
        if data_mask & DM_BIT_T_WATER:
            raw = raw_values[index]
            index += 1
            measurements.t_water = raw / 16
        if data_mask & DM_BIT_T_INSIDE:
            raw = raw_values[index]
            index += 1
            measurements.t_inside = -45.0 + 175.0 * (raw / (2**16))
        if data_mask & DM_BIT_RH_INSIDE:
            raw = raw_values[index]
            index += 1
            measurements.rh_inside = 100.0 * (raw / (2**16))
        if data_mask & DM_BIT_V_SUPPLY:
            raw = raw_values[index]
            index += 1
            measurements.v_supply = (raw + 2000) / 1000

    # Print data (including timestamp, for easier grepping)
    msg_parts = []
    if measurements.t_water is not None:
        msg_parts.append('Water Temp: %.2f °C' % measurements.t_water)
    if measurements.t_inside is not None:
        msg_parts.append('Inside Temp: %.2f °C' % measurements.t_inside)
    if measurements.rh_inside is not None:
        msg_parts.append('Inside Humi: %.2f %%RH' % measurements.rh_inside)
    if measurements.v_supply is not None:
        msg_parts.append('Voltage: %.2f V' % measurements.v_supply)
    if len(msg_parts) == 0:
        msg_parts.append('No data')
    print('  Decoded (%s): %s' % (datetime.datetime.now().isoformat(), ' | '.join(msg_parts)))

    # Determine API sensor ID
    sensor_id = SENSOR_MAPPINGS.get(dev_eui)
    if sensor_id is None:
        print('Error: No sensor mapping found for DevEUI %s' % dev_eui)
        return

    # Send to API
    if measurements.t_water is not None:
        send_to_api(sensor_id, measurements.t_water, {
            'enclosure_temp': measurements.t_inside,
            'enclosure_humi': measurements.rh_inside,
            'voltage': measurements.v_supply,
            'protocol_version': protocol_version,
        })
    else:
        print('No water temperature measurement, not sending to API')

    # Log to InfluxDB
    fields = {}
    fields['max_rssi'] = int(max_rssi)
    fields['max_snr'] = float(max_snr)
    if measurements.t_water is not None:
        fields['water_temp'] = float(measurements.t_water)
    if measurements.t_inside is not None:
        fields['enclosure_temp'] = float(measurements.t_inside)
    if measurements.rh_inside is not None:
        fields['enclosure_humi'] = float(measurements.rh_inside)
    if measurements.v_supply is not None:
        fields['voltage'] = float(measurements.v_supply)
    tags = {
        'sensor_id': sensor_id,
        'dev_id': device_id,
        'dev_eui': dev_eui,
        'sf': spreading_factor,
        'bw': bandwidth_khz,
        'best_gateway': best_gateway_id,
        'protocol_version': protocol_version,
    }
    print('Logging to InfluxDB...')
    log_to_influxdb(influxdb_client, fields, tags)

    print()


ttn_client = mqtt.Client()
ttn_client.on_connect = on_connect
ttn_client.on_message = on_message

ttn_client.username_pw_set(TTN_MQTT_USERNAME, TTN_MQTT_PASSWORD)
ttn_client.tls_set('mqtt-ca.pem', tls_version=ssl.PROTOCOL_TLSv1_2)
ttn_client.connect(TTN_MQTT_ENDPOINT, 8883, 60)

influxdb_client.write_points([{
    'measurement': 'startup',
    'fields': {
        'value': int(time.time()),  # Make it unique
    },
    'tags': {
        'service': 'gfroerli-relay',
    }
}])

ttn_client.loop_forever()
