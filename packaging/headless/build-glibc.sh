#!/bin/sh
# Builds the headless editions for glibc Linux on an old base (AlmaLinux 8, glibc 2.28), so that the programs run
# on AlmaLinux / Rocky / RHEL 8+, Debian 10+, Ubuntu 20.04+, Raspberry Pi OS (64-bit) and Fedora:
#   docker run --rm -v "$PWD":/src -w /src almalinux:8 sh packaging/headless/build-glibc.sh
set -eu
dnf install -y -q gcc gcc-c++ make cmake perl pkgconf-pkg-config alsa-lib-devel curl >/dev/null
if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
fi
. "$HOME/.cargo/env"
export CARGO_TARGET_DIR=target/glibc
cargo build --release --no-default-features --bin radiomoon-mpx-txd --bin radiomoon-mpx-rxd
ls -l target/glibc/release/radiomoon-mpx-txd target/glibc/release/radiomoon-mpx-rxd
# the newest glibc symbol the programs need
objdump -T target/glibc/release/radiomoon-mpx-rxd 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n1 || true
