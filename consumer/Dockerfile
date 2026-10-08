FROM rust:1-bookworm AS builder
ARG BIN

RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/src/app/target \
    cargo build --release --bin ${BIN} \
    && cp target/release/${BIN} /usr/local/bin/consumer

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY --from=builder /usr/local/bin/consumer /usr/local/bin/

EXPOSE 9091
ENTRYPOINT ["consumer"]
