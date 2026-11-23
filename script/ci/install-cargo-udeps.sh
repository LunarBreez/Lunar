#!/usr/bin/env bash
# Installe de quoi chercher les dépendances inutilisées : la toolchain nightly
# exigée par cargo-udeps (canal épinglé dans script/ci/check-unused-deps.sh)
# et le binaire officiel de cargo-udeps, version et SHA-256 épinglés, dans le
# répertoire donné (par défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.1.61"
sha256="27031ca3902390a1d793901d17ec073ac85297ab3477b13faea984e4c97de479"
dest="${1:-$HOME/.cargo/bin}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
channel="$("$root/script/ci/check-unused-deps.sh" --print-toolchain)"
[[ -n "$channel" ]] || { echo "canal nightly de cargo-udeps introuvable" >&2; exit 1; }
rustup toolchain install "$channel" --profile minimal

name="cargo-udeps-v$version-x86_64-unknown-linux-musl"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL --retry 3 -o "$tmp/$name.tar.gz" \
  "https://github.com/est31/cargo-udeps/releases/download/v$version/$name.tar.gz"
echo "$sha256  $tmp/$name.tar.gz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tar.gz" >&2; exit 1; }
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
install -D -m 0755 "$tmp/$name/cargo-udeps" "$dest/cargo-udeps"
echo "cargo-udeps $version, toolchain $channel"
