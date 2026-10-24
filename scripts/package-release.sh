#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Uso: $0 VERSÃO PASTA_DE_SAÍDA" >&2
    echo "Empacota target/release/readrss em readrss-VERSÃO-linux-x86_64.tar.gz + SHA256SUMS." >&2
    exit 2
}

[[ $# -eq 2 ]] || usage
version="$1"
out="$(mkdir -p "$2" && cd "$2" && pwd)"
root="$(cd "$(dirname "$0")/.." && pwd)"
name="readrss-$version-linux-x86_64"
stage="$out/$name"

[[ -x "$root/target/release/readrss" ]] || { echo "Erro: rode cargo build --release antes." >&2; exit 1; }

rm -rf "$stage"
install -Dm755 "$root/target/release/readrss" "$stage/bin/readrss"
mkdir -p "$stage/share/readrss"
cp -R "$root/data/locale" "$root/data/icons" "$stage/share/readrss/"
install -Dm755 "$root/scripts/install.sh" "$stage/install.sh"
install -Dm644 "$root/LICENSE" "$stage/LICENSE"
install -Dm644 "$root/README.md" "$stage/README.md"

mtime="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct 2>/dev/null || date +%s)}"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$mtime" \
    -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"
(cd "$out" && sha256sum "$name.tar.gz" > SHA256SUMS)
echo "$out/$name.tar.gz"
