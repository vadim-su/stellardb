# The builder always runs on the native build platform and cross-compiles to
# $TARGETARCH, so multi-arch images do not pay for QEMU-emulated rustc/LLVM.
# bun (used by build.rs to bundle Station) must also be a build-platform binary;
# a bare `COPY --from=oven/bun:1` would resolve to the target platform.
FROM --platform=$BUILDPLATFORM oven/bun:1 AS bun
FROM --platform=$BUILDPLATFORM rust:1-trixie AS builder
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun

ARG TARGETARCH
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config clang cmake \
    && if [ "$TARGETARCH" = "arm64" ]; then \
         apt-get install -y --no-install-recommends \
           gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross; \
       fi \
    && rm -rf /var/lib/apt/lists/*

RUN case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && echo "$target" > /rust-target \
    && rustup target add "$target"

# aws-lc-sys, ring, zstd-sys, and numkong/usearch compile C/C++; point their
# toolchain at the cross gcc. trixie's gcc-14 is required: bookworm's gcc-12
# rejects numkong's NEON dot-product kernels ("inlining failed ... vdotq_s32").
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar

COPY . .

RUN target="$(cat /rust-target)" \
    && cargo build --release --locked --no-default-features \
       --features cli,server,auth,tls,fts,hnsw,ui --bin stellar --target "$target" \
    && cp "target/$target/release/stellar" /stellar

FROM debian:trixie-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /stellar /usr/local/bin/stellar

EXPOSE 3000
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/stellar"]
# The base image is service-to-service/default-deny CORS. Compose may replace this
# with an explicit STELLARDB_CORS_ALLOW_ORIGINS for browser clients.
CMD ["serve", "--host", "0.0.0.0", "--insecure-allow-remote", "--non-browser-api", "--data", "/data"]
