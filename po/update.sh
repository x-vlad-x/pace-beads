#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Regenerates the template from the sources and merges it into every catalog.
# Requires GNU gettext 0.25 or newer for Rust extraction.
# Usage: po/update.sh [--check]
#   --check  fails when the catalogs are out of date or incomplete, without
#            changing them.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

xgettext --from-code=UTF-8 --language=Rust \
    --keyword --keyword=gettext --keyword=ngettext:1,2 --keyword=pgettext:1c,2 \
    --package-name=pace-beads --no-wrap --add-location=file \
    --files-from=po/POTFILES --output="$work/pace-beads.pot"
# The creation date would change on every run.
sed -i '/^"POT-Creation-Date:/d' "$work/pace-beads.pot"

status=0
if [ "${1:-}" = "--check" ]; then
    if ! grep -v '^#' "$work/pace-beads.pot" | cmp -s - <(grep -v '^#' po/pace-beads.pot) 2>/dev/null; then
        echo "po/pace-beads.pot is out of date" >&2
        status=1
    fi
    for catalog in po/*.po; do
        msgcmp "$catalog" "$work/pace-beads.pot" || status=1
        msgfmt --check --statistics --output-file=/dev/null "$catalog" || status=1
        # msgcmp accepts entries the sources no longer use.
        msgmerge --quiet --no-wrap --no-fuzzy-matching --output-file="$work/merged.po" "$catalog" "$work/pace-beads.pot"
        if grep -q '^#~' "$work/merged.po"; then
            echo "$catalog has entries the sources no longer use" >&2
            status=1
        fi
    done
    exit "$status"
fi

cp "$work/pace-beads.pot" po/pace-beads.pot
for catalog in po/*.po; do
    msgmerge --quiet --no-wrap --no-fuzzy-matching --update --backup=none "$catalog" po/pace-beads.pot
    msgattrib --no-wrap --no-obsolete --output-file="$catalog" "$catalog"
done
