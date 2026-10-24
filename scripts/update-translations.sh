#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo run --quiet --package extract-strings -- po/readrss.pot src/*.rs

while read -r lang; do
    [[ -z "$lang" || "$lang" == \#* ]] && continue
    po="po/$lang.po"
    msgmerge --quiet --update --backup=none --no-fuzzy-matching "$po" po/readrss.pot
    msgattrib --no-obsolete --output-file="$po" "$po"
    out="data/locale/$lang/LC_MESSAGES"
    mkdir -p "$out"
    msgfmt --check --statistics --output-file="$out/readrss.mo" "$po"
done < po/LINGUAS
