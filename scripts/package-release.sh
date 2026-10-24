#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Uso: $0 VERSÃO PASTA_DE_SAÍDA" >&2
    echo "Empacota target/release/readrss em readrss-VERSÃO-linux-x86_64.tar.gz e readrss_VERSÃO-1_amd64.deb," >&2
    echo "mais o SHA256SUMS dos dois. Requer cargo-deb." >&2
    exit 2
}

[[ $# -eq 2 ]] || usage
version="$1"
out="$(mkdir -p "$2" && cd "$2" && pwd)"
root="$(cd "$(dirname "$0")/.." && pwd)"
name="readrss-$version-linux-x86_64"
stage="$out/$name"
extra="$root/target/debian-extra"

[[ -x "$root/target/release/readrss" ]] || { echo "Erro: rode cargo build --release antes." >&2; exit 1; }
command -v cargo-deb >/dev/null || { echo "Erro: cargo-deb não encontrado (cargo install --locked cargo-deb)." >&2; exit 1; }

mtime="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct 2>/dev/null || date +%s)}"
maintainer="$(sed -n 's/^maintainer = "\(.*\)"$/\1/p' "$root/Cargo.toml")"

rm -rf "$stage"
install -Dm755 "$root/target/release/readrss" "$stage/bin/readrss"
mkdir -p "$stage/share/readrss"
cp -R "$root/data/locale" "$root/data/icons" "$root/data/io.github.readrss.ReadRSS.desktop" \
    "$root/data/readrss.service" "$stage/share/readrss/"
install -Dm755 "$root/scripts/install.sh" "$stage/install.sh"
install -Dm644 "$root/LICENSE" "$stage/LICENSE"
install -Dm644 "$root/README.md" "$stage/README.md"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$mtime" \
    -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"

mkdir -p "$extra"
gzip -9n < "$root/data/readrss.1" > "$extra/readrss.1.gz"
cat > "$extra/changelog" <<EOF
readrss ($version-1) stable; urgency=medium

  * Release $version. The full list of changes is in the app: menu → Changelog,
    and at https://github.com/EmersonComar/readrss/releases/tag/v$version

 -- $maintainer  $(LC_ALL=C date -R -u -d "@$mtime")
EOF
cargo deb --package readrss --no-build --deb-version "$version-1" --output "$out" >/dev/null

(cd "$out" && sha256sum "$name.tar.gz" "readrss_${version}-1_amd64.deb" > SHA256SUMS)
ls "$out"/*.tar.gz "$out"/*.deb
