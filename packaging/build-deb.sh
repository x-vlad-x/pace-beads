#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Usage: build-deb.sh <directory with release binaries>
# Runs inside the cross image, from the repository root.
set -eu
binaries=$1
upstream=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
revision=${DEB_REVISION:-1}
version=$upstream-$revision
output=target/deb
root=$output/root
timestamp=${SOURCE_DATE_EPOCH:-$(stat -c %Y Cargo.lock)}
export SOURCE_DATE_EPOCH=$timestamp

rm -rf "$root"
mkdir -p "$root/DEBIAN"
sh packaging/stage.sh "$binaries" "$root" /usr

doc=$root/usr/share/doc/pace-beads
mkdir -p "$doc"
install -m644 packaging/copyright "$doc/copyright"
install -m644 "$output/THIRD-PARTY-LICENSES" "$doc/THIRD-PARTY-LICENSES"
gzip -9nc CHANGELOG.md > "$doc/changelog.gz"

# dpkg-shlibdeps insists on a debian/control next to the working directory.
scratch=$output/shlibs
rm -rf "$scratch"
mkdir -p "$scratch/debian"
: > "$scratch/debian/control"
absolute=$(pwd)/$root
shlibs=$(cd "$scratch" && dpkg-shlibdeps -O -e"$absolute/usr/bin/pace-beads" -e"$absolute/usr/bin/pace-beads-recorder" \
    | sed -n 's/^shlibs:Depends=//p')
[ -n "$shlibs" ] || { echo "dpkg-shlibdeps found no dependencies" >&2; exit 1; }
size=$(du -sk --exclude=DEBIAN "$root" | cut -f1)

cat > "$root/DEBIAN/control" <<CONTROL
Package: pace-beads
Version: $version
Architecture: arm64
Maintainer: Vladislav Shadiuk <6992531+x-vlad-x@users.noreply.github.com>
Installed-Size: $size
Section: utils
Priority: optional
Depends: $shlibs, hicolor-icon-theme, init-system-helpers (>= 1.66), sensorfw-qt6, libsensorfw-qt6-hybris
Homepage: https://github.com/x-vlad-x/pace-beads
Description: Offline step counter for Linux phones
 Records the hardware step counter through SensorFW and shows goals,
 streaks, milestones and history. Data stays on the device.
 .
 This preview release supports FuriOS on the FuriPhone FLX1s.
CONTROL
install -m755 packaging/postinst "$root/DEBIAN/postinst"
install -m755 packaging/prerm "$root/DEBIAN/prerm"
install -m755 packaging/postrm "$root/DEBIAN/postrm"

find "$root" -exec touch -h -d "@$timestamp" {} +
dpkg-deb --root-owner-group --build "$root" "$output/pace-beads_${version}_arm64.deb"
