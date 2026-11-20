#!/usr/bin/env bash
# Installe la toolchain de rust-toolchain.toml avec clippy et rustfmt, sans
# dépendre de l'installation implicite de rustup.
set -Eeuo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
channel="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")"
[[ -n "$channel" ]] || { echo "canal introuvable dans rust-toolchain.toml" >&2; exit 1; }
rustup toolchain install "$channel" --profile minimal --component clippy --component rustfmt
rustc --version
cargo --version
