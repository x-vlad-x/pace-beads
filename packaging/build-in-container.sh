#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
set -eu
export CARGO_TARGET_DIR=/src/target/cross
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
cargo build --release --locked --target aarch64-unknown-linux-gnu
cargo test --release --locked --target aarch64-unknown-linux-gnu --no-run > target/cross/test-binaries.txt 2>&1 ||
    { cat target/cross/test-binaries.txt; exit 1; }
cat target/cross/test-binaries.txt
sh packaging/third-party-licenses.sh target/deb/THIRD-PARTY-LICENSES
sh packaging/build-deb.sh "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release"
