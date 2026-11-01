#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Collects the license texts of every crate linked into the arm64 binaries.
# Usage: third-party-licenses.sh <output file>
# Fails when a crate ships no license text, so nothing is omitted silently.
set -eu
output=$1
target=aarch64-unknown-linux-gnu
registry=${CARGO_HOME:-$HOME/.cargo}/registry/src
mkdir -p "$(dirname "$output")"

{
    echo "Third-party components of the Pace Beads binaries"
    echo
    echo "Generated from Cargo.lock for $target. The application itself is"
    echo "GPL-3.0-only. SQLite is compiled in through libsqlite3-sys and is in"
    echo "the public domain."
    echo
} > "$output"

cargo tree --locked --target "$target" --edges normal --prefix none --format '{p}|{l}' > "$output.tree"
sed 's/ (\*)$//; s/ (proc-macro)//' "$output.tree" | grep -v -e '^pace-' -e '^$' | sort -u > "$output.list"
rm -f "$output.tree"
[ -s "$output.list" ] || { echo "cargo tree listed no crates" >&2; exit 1; }

missing=0
while IFS='|' read -r package license; do
    name=${package% v*}
    version=${package##* v}
    version=${version%% *}
    directory=$(ls -d "$registry"/*/"$name-$version" 2>/dev/null | head -n1 || true)
    : > "$output.files"
    if [ -n "$directory" ]; then
        find "$directory" -maxdepth 1 -type f \
            \( -iname 'LICENSE*' -o -iname 'LICENCE*' -o -iname 'COPYING*' -o -iname 'NOTICE*' -o -iname 'UNLICENSE*' \) \
            | sort > "$output.files"
    fi
    if [ ! -s "$output.files" ]; then
        echo "no license text for $name $version ($license)" >&2
        missing=1
        continue
    fi
    {
        echo "================================================================"
        echo "$name $version ($license)"
        echo "================================================================"
        while IFS= read -r file; do
            echo "--- $(basename "$file") ---"
            cat "$file"
            echo
        done < "$output.files"
    } >> "$output"
done < "$output.list"
rm -f "$output.files"
rm -f "$output.list"
exit "$missing"
