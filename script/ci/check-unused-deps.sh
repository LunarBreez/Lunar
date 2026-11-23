#!/usr/bin/env bash
# Dépendances déclarées mais inutilisées (cargo-udeps), sur tout le workspace
# et toutes ses cibles. cargo-udeps exige une toolchain nightly, épinglée ici.
# Ce n'est pas celle du fuzzing : filetime, tiré par gpui, appelle
# `fs::set_times` dès que rustc s'annonce en 1.99, et la fonction n'était pas
# encore stable dans le nightly 1.99 du fuzzing.
#
# Usage : check-unused-deps.sh [options de cargo udeps]
#         check-unused-deps.sh --print-toolchain
# Exige cargo-udeps et la toolchain (script/ci/install-cargo-udeps.sh).
set -Eeuo pipefail
channel="nightly-2026-09-01"
if [[ "${1:-}" == "--print-toolchain" ]]; then
  echo "$channel"
  exit 0
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
cargo "+$channel" udeps --workspace --all-targets --locked "$@"
