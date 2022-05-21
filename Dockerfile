# Build
FROM rust:1-slim-bullseye AS builder
COPY . /src
RUN apt-get update \
 && apt-get install -y cmake pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
RUN cd /src && cargo build --release

# Create runtime container
# Note that we need a small init process for PID 1 that forwards signals.
# See https://github.com/Yelp/dumb-init
FROM debian:11-slim
RUN apt-get update && apt-get install -y dumb-init && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/ttn-relay /usr/local/bin/
RUN addgroup --gid 2343 relay \
 && adduser --disabled-password --gecos "" --uid 2343 --gid 2343 relay \
 && chown relay:relay /usr/local/bin/ttn-relay
USER relay
ENTRYPOINT ["/usr/bin/dumb-init", "--"]
CMD [ "ttn-relay", "--config", "/etc/ttn-relay.toml" ]
