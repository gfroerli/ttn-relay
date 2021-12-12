[![CI][ci-badge]][ci]
[![Docker][docker-badge]][docker]

# TTN Relay

A Python 3 script to relay data from The Things Network (v3) to our own
application server (and to InfluxDB).

## Configuration

Set the following env variables:

- `DEBUG`: Enable debugging mode
- `TTN_MQTT_ENDPOINT`: Optional MQTT endpoint (hostname)
- `TTN_MQTT_USERNAME`: The TTN MQTT username
- `TTN_MQTT_PASSWORD`: The TTN MQTT password
- `API_TOKEN`: The Water Sensor API token (with write access)
- `SENSOR_MAPPINGS`: A comma separated list of (DevEUI, SensorID) pairs.
  Example: `0004A30B001FAAAA,4,0004A30B001FBBBB,5`

You can also place those env variables in an `.env` file, they will be read
automatically.

## Docker

A docker image is built at
[gfroerli/ttn-relay](https://hub.docker.com/r/gfroerli/ttn-relay/)
for every push to master.

<!-- Badges -->
[ci]: https://github.com/gfroerli/ttn-relay/actions?query=workflow%3ACI
[ci-badge]: https://img.shields.io/github/workflow/status/gfroerli/ttn-relay/CI/master
[docker]: https://hub.docker.com/r/gfroerli/ttn-relay/
[docker-badge]: https://img.shields.io/badge/docker%20image-gfroerli%2Fttn--relay-blue.svg
