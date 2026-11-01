#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Builds arm64 binaries and a .deb inside a Debian forky cross image.
# Usage: packaging/cross-build.sh [image]
set -eu
image=${1:-localhost/glissando-cross:forky}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
# The cross image has no gettext, so catalogs are compiled on the host.
mkdir -p "$root/target/locale"
for catalog in "$root"/po/*.po; do
    msgfmt -o "$root/target/locale/$(basename "$catalog" .po).mo" "$catalog"
done
podman run --rm -v "$root:/src:Z" -v "$HOME/.cargo/registry:/root/.cargo/registry:Z" \
    -w /src "$image" sh -eu packaging/build-in-container.sh
