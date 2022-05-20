[![CI][ci-badge]][ci]
[![Docker][docker-badge]][docker]

# TTN Relay

A Rust program to relay data from The Things Network (v3) to our own
application server (and to InfluxDB).

## Configuration

Copy `config.toml.example` to `config.toml` and adjust it.

Then run `ttn-relay` with `--config <path-to-config.toml>`.

## Docker

A docker image is built at
[gfroerli/ttn-relay](https://hub.docker.com/r/gfroerli/ttn-relay/)
for every push to master.

<!-- Badges -->
[ci]: https://github.com/gfroerli/ttn-relay/actions?query=workflow%3ACI
[ci-badge]: https://img.shields.io/github/workflow/status/gfroerli/ttn-relay/CI/master
[docker]: https://hub.docker.com/r/gfroerli/ttn-relay/
[docker-badge]: https://img.shields.io/badge/docker%20image-gfroerli%2Fttn--relay-blue.svg
