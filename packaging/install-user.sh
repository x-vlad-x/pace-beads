#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Per-user installation for testing, without system packages.
# Usage: install-user.sh <unpacked staging root with ./.local>
#        install-user.sh --uninstall
set -eu
app_id=io.github.x_vlad_x.PaceBeads
user_units=$HOME/.config/systemd/user

# A stale cache hides new icons: GTK trusts it while the theme directory's
# own date is unchanged, which adding a file to a subdirectory does not touch.
refresh_icon_cache() {
    gtk-update-icon-cache -f -t -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
}

if [ "${1:-}" = "--uninstall" ]; then
    systemctl --user disable --now pace-beads-recorder.service 2>/dev/null || true
    rm -f "$HOME/.local/bin/pace-beads" "$HOME/.local/bin/pace-beads-recorder" \
        "$user_units/pace-beads-recorder.service" \
        "$HOME/.local/share/applications/$app_id.desktop" \
        "$HOME/.local/share/metainfo/$app_id.metainfo.xml" \
        "$HOME/.local/share/icons/hicolor/scalable/apps/$app_id.svg" \
        "$HOME/.local/share/icons/hicolor/symbolic/apps/$app_id-symbolic.svg" \
        "$HOME/.local/share/dbus-1/services/$app_id.Recorder.service" \
        "$HOME/.local/share/locale/de/LC_MESSAGES/pace-beads.mo" \
        "$HOME/.local/share/locale/ru/LC_MESSAGES/pace-beads.mo"
    refresh_icon_cache
    systemctl --user daemon-reload
    echo "Removed. History stays in ~/.local/share/pace-beads."
    exit 0
fi

staged=$1
# A running executable cannot be overwritten, but it can be replaced.
rm -f "$HOME/.local/bin/pace-beads" "$HOME/.local/bin/pace-beads-recorder"
cp -r "$staged/.local/." "$HOME/.local/"
refresh_icon_cache
mkdir -p "$user_units"
mv "$HOME/.local/lib/systemd/user/pace-beads-recorder.service" "$user_units/"
sed -i "s|=/.local/bin|=$HOME/.local/bin|" "$user_units/pace-beads-recorder.service" \
    "$HOME/.local/share/dbus-1/services/$app_id.Recorder.service"
sed -i "s|^Exec=pace-beads$|Exec=$HOME/.local/bin/pace-beads|" "$HOME/.local/share/applications/$app_id.desktop"
systemctl --user daemon-reload
systemctl --user enable --now pace-beads-recorder.service
systemctl --user restart pace-beads-recorder.service
