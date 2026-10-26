#!/bin/sh
# Builds the headless editions on Alpine Linux (musl), e.g. in a container:
#   docker run --rm -v "$PWD":/src -w /src alpine:3.20 sh packaging/headless/build-alpine.sh
set -eu
apk add --no-cache build-base cmake pkgconf alsa-lib-dev linux-headers curl perl >/dev/null
if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
fi
. "$HOME/.cargo/env"
# dynamic musl: libasound comes from the system (apk add alsa-lib)
export RUSTFLAGS="-C target-feature=-crt-static"
export CARGO_TARGET_DIR=target/alpine
cargo build --release --no-default-features --bin radiomoon-mpx-txd --bin radiomoon-mpx-rxd
# the CLAP plug-in for audio programs on this C library (e.g. radiomoon-server)
cargo build --release -p radiomoon_mpx_clap
cp target/alpine/release/libradiomoon_mpx_clap.so target/alpine/release/RadioMoon-MPX-Transmitter.clap
ls -l target/alpine/release/radiomoon-mpx-txd target/alpine/release/radiomoon-mpx-rxd target/alpine/release/RadioMoon-MPX-Transmitter.clap
