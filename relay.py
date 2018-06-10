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

import dotenv
import paho.mqtt.client as mqtt
import requests


# Config from .env file
dotenv.load_dotenv(dotenv.find_dotenv())

# General
DEBUG = os.environ.get('DEBUG', '0').lower() in ['1', 'true', 'yes', 'y']

# TTN
TTN_APP_ID = os.environ.get('TTN_APP_ID')
TTN_ACCESS_KEY = os.environ.get('TTN_ACCESS_KEY')

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


def send_to_api(sensor_id: int, temperature: float, attributes: dict):
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


def on_connect(client, userdata, flags, rc):
    print("Connected with result code %s: %s" % (rc, CONNECT_RETURN_CODES.get(rc, 'Unknown')))
    if rc == 0:
        client.subscribe('+/devices/+/activations')
        client.subscribe('+/devices/+/up')


def on_message(client, userdata, msg):
    print('\n%s' % msg.topic)
    data = json.loads(msg.payload.decode('utf8'))
    if DEBUG:
        pprint(data)
    print('Message details:')
    print('  Dev ID: %s' % data['dev_id'])
    print('  Dev EUI: %s' % data['hardware_serial'])
    print('  Data rate: %s' % data['metadata']['data_rate'])
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


client = mqtt.Client()
client.on_connect = on_connect
client.on_message = on_message

client.username_pw_set(TTN_APP_ID, TTN_ACCESS_KEY)
client.tls_set('mqtt-ca.pem', tls_version=ssl.PROTOCOL_TLSv1_2)
client.connect('eu.thethings.network', 8883, 60)

client.loop_forever()
