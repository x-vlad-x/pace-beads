#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Stages installed files: stage.sh <binary dir> <staging root> <prefix>
set -eu
binaries=$1
root=$2
prefix=$3
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_id=io.github.x_vlad_x.PaceBeads

install -Dm755 "$binaries/pace-beads" "$root$prefix/bin/pace-beads"
install -Dm755 "$binaries/pace-beads-recorder" "$root$prefix/bin/pace-beads-recorder"
install -Dm644 "$source_dir/data/$app_id.desktop" "$root$prefix/share/applications/$app_id.desktop"
install -Dm644 "$source_dir/data/$app_id.metainfo.xml" "$root$prefix/share/metainfo/$app_id.metainfo.xml"
install -Dm644 "$source_dir/design/pace-beads.svg" "$root$prefix/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 "$source_dir/design/pace-beads-symbolic.svg" "$root$prefix/share/icons/hicolor/symbolic/apps/$app_id-symbolic.svg"

mkdir -p "$root$prefix/lib/systemd/user" "$root$prefix/share/dbus-1/services"
sed "s|@BINDIR@|$prefix/bin|" "$source_dir/data/pace-beads-recorder.service" \
    > "$root$prefix/lib/systemd/user/pace-beads-recorder.service"
sed "s|@BINDIR@|$prefix/bin|" "$source_dir/data/$app_id.Recorder.service" \
    > "$root$prefix/share/dbus-1/services/$app_id.Recorder.service"

for catalog in "$source_dir"/po/*.po; do
    language=$(basename "$catalog" .po)
    mkdir -p "$root$prefix/share/locale/$language/LC_MESSAGES"
    if command -v msgfmt >/dev/null; then
        msgfmt -o "$root$prefix/share/locale/$language/LC_MESSAGES/pace-beads.mo" "$catalog"
    else
        cp "$source_dir/target/locale/$language.mo" "$root$prefix/share/locale/$language/LC_MESSAGES/pace-beads.mo"
    fi
done
