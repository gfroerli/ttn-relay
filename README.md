# TTN Relay

A Python 3 script to relay data from The Things Network to our own application
server.

## Configuration

Set the following env variables:

- `DEBUG`: Enable debugging mode
- `TTN_APP_ID`: The TTN App ID
- `TTN_ACCESS_KEY`: The TTN Access Key
- `API_TOKEN`: The Water Sensor API token (with write access)
- `SENSOR_MAPPINGS`: A comma separated list of (DevEUI, SensorID) pairs.
  Example: `0004A30B001FAAAA,4,0004A30B001FBBBB,5`

You can also place those env variables in an `.env` file, they will be read
automatically.

## Docker

A docker image is built at
[gfroerli/ttn-relay](https://hub.docker.com/r/gfroerli/ttn-relay/)
for every push to master.
