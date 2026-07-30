FROM rust:1.88-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release --bin server --bin client --bin api_gateway

FROM debian:bookworm-slim AS runtime

RUN groupadd --system --gid 10001 hogyoku \
    && useradd --system --uid 10001 --gid hogyoku --home-dir /app --create-home hogyoku \
    && mkdir -p /data \
    && chown hogyoku:hogyoku /data

COPY --from=builder /build/target/release/server /usr/local/bin/server
COPY --from=builder /build/target/release/client /usr/local/bin/client
COPY --from=builder /build/target/release/api_gateway /usr/local/bin/api_gateway
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

USER 10001:10001
WORKDIR /app
VOLUME ["/data"]
EXPOSE 8000 8080 8081

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
