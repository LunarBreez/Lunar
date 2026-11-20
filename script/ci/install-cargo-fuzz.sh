#!/usr/bin/env bash
# Installe de quoi fuzzer : la toolchain nightly exigée par cargo-fuzz (canal
# de crates/lunar-flows-core/fuzz/rust-toolchain.toml) et le binaire officiel
# de cargo-fuzz, version et SHA-256 épinglés, dans le répertoire donné (par
# défaut ~/.cargo/bin).
set -Eeuo pipefail
version="0.13.2"
sha256="b5b704018b63e0f151c17a057ac53b5111e1db545d1b9f72fee79f08a545931c"
dest="${1:-$HOME/.cargo/bin}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
toolchain_file="$root/crates/lunar-flows-core/fuzz/rust-toolchain.toml"
channel="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$toolchain_file")"
[[ -n "$channel" ]] || { echo "canal introuvable dans $toolchain_file" >&2; exit 1; }
rustup toolchain install "$channel" --profile minimal

name="cargo-fuzz-$version-x86_64-unknown-linux-musl"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL --retry 3 -o "$tmp/$name.tar.gz" \
  "https://github.com/rust-fuzz/cargo-fuzz/releases/download/$version/$name.tar.gz"
echo "$sha256  $tmp/$name.tar.gz" | sha256sum --check --status ||
  { echo "SHA-256 inattendu pour $name.tar.gz" >&2; exit 1; }
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
install -D -m 0755 "$tmp/cargo-fuzz" "$dest/cargo-fuzz"
"$dest/cargo-fuzz" --version
