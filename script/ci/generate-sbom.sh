#!/usr/bin/env bash
# Génère les SBOM CycloneDX 1.5 (JSON) du binaire lunar, un par cible livrée,
# dans le répertoire donné : lunar-<étiquette>-<cible>.cdx.json.
# Usage : generate-sbom.sh <étiquette> [répertoire]. Exige cargo-cyclonedx
# (script/ci/install-cargo-cyclonedx.sh).
set -Eeuo pipefail
label="${1:?usage: generate-sbom.sh <étiquette> [répertoire]}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$(mkdir -p "${2:-dist}" && cd "${2:-dist}" && pwd)"
cd "$root"

# cargo-cyclonedx écrit un fichier à côté du Cargo.toml de chaque crate du
# workspace : seul celui de lunar, qui couvre toutes ses dépendances, est gardé.
generate() {
  local triple="$1" suffix="$2" stem="lunar-sbom-tmp.cdx"
  cargo cyclonedx --manifest-path crates/lunar/Cargo.toml --format json \
    --spec-version 1.5 --all --target "$triple" --override-filename "$stem" --quiet
  mv "crates/lunar/$stem.json" "$out/lunar-$label-$suffix.cdx.json"
  rm -f crates/*/"$stem.json"
}

generate x86_64-unknown-linux-gnu x86_64-linux-gnu
generate x86_64-pc-windows-msvc x86_64-windows
ls -l "$out"/*.cdx.json
