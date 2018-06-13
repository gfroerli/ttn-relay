"""
Water Sensor Project: Relay from TTN to API.
"""
from pprint import pprint
import base64
import datetime
import json
import os
import ssl
import struct
import sys
import time
from typing import Any, Dict

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
TTN_APP_ID = require_env('TTN_APP_ID')
TTN_ACCESS_KEY = require_env('TTN_ACCESS_KEY')

# InfluxDB
INFLUXDB_HOST = require_env('INFLUXDB_HOST')
INFLUXDB_PORT = require_env('INFLUXDB_PORT')
INFLUXDB_USER = require_env('INFLUXDB_USER')
INFLUXDB_PASS = require_env('INFLUXDB_PASS')
INFLUXDB_DB = require_env('INFLUXDB_DB')

# Payload
PAYLOAD_FORMAT = '<ffff'

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


# Create InfluxDB client
influxdb_client = influxdb.InfluxDBClient(
    INFLUXDB_HOST, INFLUXDB_PORT,
    INFLUXDB_USER, INFLUXDB_PASS,
    INFLUXDB_DB,
    ssl=True, verify_ssl=True, timeout=10,
)


def send_to_api(
    sensor_id: int,
    temperature: float,
    attributes: dict,
):
    """
    Send temperature measurement to API.

    TODO: Send along atributes
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
        print('OK')
    else:
        print('HTTP%d' % resp.status_code)


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
        client.subscribe('+/devices/+/activations')
        client.subscribe('+/devices/+/up')


def on_message(client, userdata, msg):
    print('\n%s' % msg.topic)
    data = json.loads(msg.payload.decode('utf8'))
    if DEBUG:
        pprint(data)

    # Get general information
    dev_id = data['dev_id']
    dev_eui = data['hardware_serial']
    data_rate = data['metadata']['data_rate']

    # Get max RSSI/SNR values
    gateways = [
        g for g in data['metadata']['gateways']
        if g.get('rssi') is not None and g.get('snr') is not None
    ]
    all_rssi = [g['rssi'] for g in gateways]
    all_snr = [g['snr'] for g in gateways]
    max_rssi = max(all_rssi) if len(all_rssi) > 0 else None
    max_snr = max(all_snr) if len(all_snr) > 0 else None

    # Get gateway with best reception
    best_gateway = sorted(gateways, key=lambda g: g['rssi'], reverse=True)[0]
    best_gateway_id = best_gateway['gtw_id']

    print('Message details:')
    print('  Dev ID: %s' % dev_id)
    print('  Dev EUI: %s' % dev_eui)
    print('  Data rate: %s' % data_rate)
    print('  Receiving gateways:')
    for gw in data['metadata']['gateways']:
        print('    - ID: %s' % gw['gtw_id'])
        print('      Coords: %s,%s' % (gw.get('latitude', '-'), gw.get('longitude', '-')))
        print('      Alt: %sm' % gw.get('altitude', '-'))
        print('      RSSI: %s' % gw.get('rssi', '-'))
        print('      SNR: %s' % gw.get('snr', '-'))

    if msg.topic.endswith('/up'):
        # Uplink message
        payload = data.get('payload_raw')
        if payload is None:
            print('Uplink msg is missing payload')
            return

        # Decode message bytes
        bytestring = base64.b64decode(payload)
        try:
            unpacked = struct.unpack(PAYLOAD_FORMAT, bytestring)
        except struct.error as e:
            print('Invalid payload format: %s' % e)
            return
        msg = '%s | DS Temp: %.2f °C | SHT Temp: %.2f °C | SHT Humi: %.2f %%RH | Voltage: %.2f V'
        timestamp = datetime.datetime.now().isoformat()
        ds18b20_temp = unpacked[0]
        sht21_temp = unpacked[1]
        sht21_humi = unpacked[2]
        voltage = unpacked[3]
        msg_full = msg % (timestamp, ds18b20_temp, sht21_temp, sht21_humi, voltage)
        print(msg_full)

        # Determine API sensor ID
        deveui = data['hardware_serial']
        sensor_id = SENSOR_MAPPINGS.get(deveui)
        if sensor_id is None:
            print('Error: No sensor mapping found for DevEUI %s' % deveui)
            return

        # Send to API
        send_to_api(sensor_id, ds18b20_temp, {
            'enclosure_temp': sht21_temp,
            'enclosure_humi': sht21_humi,
            'voltage': voltage,
        })

        # Log to InfluxDB
        fields = {
            'water_temp': ds18b20_temp,
            'enclosure_temp': sht21_temp,
            'enclosure_humi': sht21_humi,
            'voltage': voltage,
            'max_rssi': max_rssi,
            'max_snr': max_snr,
        }
        tags = {
            'sensor_id': sensor_id,
            'dev_id': dev_id,
            'dev_eui': dev_eui,
            'datarate': data_rate,
            'best_gateway': best_gateway_id,
        }
        log_to_influxdb(influxdb_client, fields, tags)


ttn_client = mqtt.Client()
ttn_client.on_connect = on_connect
ttn_client.on_message = on_message

ttn_client.username_pw_set(TTN_APP_ID, TTN_ACCESS_KEY)
ttn_client.tls_set('mqtt-ca.pem', tls_version=ssl.PROTOCOL_TLSv1_2)
ttn_client.connect('eu.thethings.network', 8883, 60)

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
