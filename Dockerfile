FROM rust:1-bookworm AS builder
COPY --from=oven/bun:1 /usr/local/bin/bun /usr/local/bin/bun

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config clang \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release --locked --no-default-features --features cli,server,auth,tls,fts,hnsw,ui --bin stellar

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/stellar /usr/local/bin/stellar

EXPOSE 3000
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/stellar"]
# The base image is service-to-service/default-deny CORS. Compose may replace this
# with an explicit STELLARDB_CORS_ALLOW_ORIGINS for browser clients.
CMD ["serve", "--host", "0.0.0.0", "--insecure-allow-remote", "--non-browser-api", "--data", "/data"]
