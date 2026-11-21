# The relay, as a container. Built from the repository root:  docker build -t airband-relay .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release -p airband-relay
# Amazon RDS signs its servers' certificates with its own authorities; this is the list of them, for DB_CA
RUN curl -fsSL https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem -o /rds-global-bundle.pem && grep -q "BEGIN CERTIFICATE" /rds-global-bundle.pem

FROM debian:bookworm-slim
# Straight into /etc, which exists: --chmod also applies to any folder COPY has to create, and a folder with mode
# 644 cannot be entered, which is how the first deploy failed ("Permission denied" reading this file).
COPY --from=build --chmod=644 /rds-global-bundle.pem /etc/rds-global-bundle.pem
COPY --from=build /src/target/release/airband-relay /usr/local/bin/airband-relay
# with no database configured, the channel list goes in a folder; this one is writable by the unprivileged user
ENV PORT=7474 DB_CA=/etc/rds-global-bundle.pem AIRBAND_RELAY_DATA=/tmp/relay-data
USER nobody
# fail the build, not the deploy, if the relay cannot start as this user with what is in the image
RUN test -r "$DB_CA" && mkdir -p "$AIRBAND_RELAY_DATA"
EXPOSE 7474
CMD ["airband-relay"]
