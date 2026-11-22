#!/usr/bin/env bash
# Installe le binaire officiel de cargo-vet, version et SHA-256 épinglés,
# dans le répertoire donné (par défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.10.4"
sha256="e367f8ac156208711511274abab0c309e19ed0f11ef2bbe8a7b0294f824cb032"
dest="${1:-$HOME/.cargo/bin}"
name="cargo-vet-x86_64-unknown-linux-gnu"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL --retry 3 -o "$tmp/$name.tar.xz" \
  "https://github.com/mozilla/cargo-vet/releases/download/v$version/$name.tar.xz"
echo "$sha256  $tmp/$name.tar.xz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tar.xz" >&2; exit 1; }
tar -xJf "$tmp/$name.tar.xz" -C "$tmp"
install -D -m 0755 "$tmp/$name/cargo-vet" "$dest/cargo-vet"
"$dest/cargo-vet" --version 2>/dev/null || echo "cargo-vet $version"
