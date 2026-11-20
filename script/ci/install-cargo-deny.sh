#!/usr/bin/env bash
# Installe le binaire officiel de cargo-deny, version et SHA-256 épinglés,
# dans le répertoire donné (par défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.22.0"
sha256="98c56e6a408bfa5c3c60744b6015aaa6aa08f743fdeb88386d06fca035bb1786"
dest="${1:-$HOME/.cargo/bin}"
name="cargo-deny-$version-x86_64-unknown-linux-musl"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL --retry 3 -o "$tmp/$name.tar.gz" \
  "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/$name.tar.gz"
echo "$sha256  $tmp/$name.tar.gz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tar.gz" >&2; exit 1; }
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
install -D -m 0755 "$tmp/$name/cargo-deny" "$dest/cargo-deny"
"$dest/cargo-deny" --version
