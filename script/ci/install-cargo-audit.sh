#!/usr/bin/env bash
# Installe le binaire officiel de cargo-audit, version et SHA-256 épinglés,
# dans le répertoire donné (par défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.23.0"
sha256="4f01c74ed84b2dffd0b1d37012a62cdf2a92c96e25bdf8eec7fcc0d4d3eb7811"
dest="${1:-$HOME/.cargo/bin}"
name="cargo-audit-x86_64-unknown-linux-musl-v$version"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
# Le tag de la release est « cargo-audit/vX.Y.Z » : la barre oblique est
# encodée dans l'URL.
curl -sSfL --retry 3 -o "$tmp/$name.tgz" \
  "https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv$version/$name.tgz"
echo "$sha256  $tmp/$name.tgz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tgz" >&2; exit 1; }
tar -xzf "$tmp/$name.tgz" -C "$tmp"
install -D -m 0755 "$tmp/$name/cargo-audit" "$dest/cargo-audit"
"$dest/cargo-audit" audit --version
