#!/usr/bin/env bash
# Installe le binaire officiel de cargo-cyclonedx, version et SHA-256
# épinglés, dans le répertoire donné (par défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.5.9"
sha256="9bd3e599314f50810c9d98b8b68a617ff9d3cc20873968d90b29d121f6b226ff"
dest="${1:-$HOME/.cargo/bin}"
name="cargo-cyclonedx-x86_64-unknown-linux-musl"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL --retry 3 -o "$tmp/$name.tar.xz" \
  "https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/cargo-cyclonedx-$version/$name.tar.xz"
echo "$sha256  $tmp/$name.tar.xz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tar.xz" >&2; exit 1; }
tar -xJf "$tmp/$name.tar.xz" -C "$tmp"
install -D -m 0755 "$tmp/$name/cargo-cyclonedx" "$dest/cargo-cyclonedx"
"$dest/cargo-cyclonedx" cyclonedx --version
