#!/usr/bin/env bash
# Dépendances vendorisées : vendor/ contient les sources de toutes les
# dépendances, aux versions exactes des Cargo.lock du produit et du crate de
# fuzzing, pour les seules cibles livrées. Le dossier est commité et
# .cargo/config.toml y redirige crates.io : rien ne se compile depuis le réseau.
# Exige jq.
#
# Sans option : régénère vendor/ depuis crates.io pour les Cargo.lock actuels.
# --update [options de cargo update] : met à jour les dépendances, puis
#     vendor/. Exemples : `--update -p packet_parser` ; après une retouche d'un
#     Cargo.toml, `--update --workspace` (résolution minimale). Sans option,
#     tout est mis à jour, y compris les versions figées à dessein (README).
# --check : sans réseau, vendor/ couvre-t-il les deux Cargo.lock ?
# --verify : régénère vendor/ depuis crates.io et exige qu'il soit identique à
#     ce qui est commité (CI) : un fichier vendorisé retouché à la main, ou un
#     oubli de régénération, fait échouer.
set -Eeuo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fuzz="$root/crates/lunar-flows-core/fuzz"
cd "$root"

# Résout un manifeste contre vendor/ seul : échoue si un paquet y manque ou si
# sa version n'est pas celle du lock.
covered() {
  cargo metadata --offline --locked --format-version 1 --manifest-path "$1" >/dev/null
}

# Cibles livrées, les mêmes que dans deny.toml : Linux x86_64 (Ubuntu 22.04)
# et Windows 10/11 x86_64 (MSVC). Le fuzzing ne tourne que sous Linux.
targets=(x86_64-unknown-linux-gnu x86_64-pc-windows-msvc)

# Crates « nom-version » qu'un manifeste demande pour une cible, toutes
# features du workspace activées. cargo metadata voit large (il garde des
# dépendances optionnelles non activées) : un crate utile n'est jamais écarté.
needed_by() {
  cargo metadata --offline --locked --all-features --format-version 1 \
    --filter-platform "$2" --manifest-path "$1" |
    jq -r '[.resolve.nodes[].id] as $ids | .packages[]
           | select(.source != null and (.id | IN($ids[])))
           | "\(.name)-\(.version)"'
}

# Réduit un crate à une coquille : son Cargo.toml, dont cargo a besoin pour
# résoudre le Cargo.lock, et une bibliothèque vide. Plus aucune source ni
# aucun binaire précompilé ; s'il servait malgré tout, la compilation échoue.
stub() {
  local dir="$1" lib package
  lib="$(awk '/^\[lib\]/ { in_lib = 1; next } /^\[/ { in_lib = 0 }
              in_lib && /^path = / { gsub(/"/, "", $3); print $3 }' "$dir/Cargo.toml")"
  lib="${lib:-src/lib.rs}"
  package="$(jq -r '.package' "$dir/.cargo-checksum.json")"
  find "$dir" -mindepth 1 ! -name Cargo.toml -delete
  mkdir -p "$dir/$(dirname "$lib")"
  : >"$dir/$lib"
  jq -n --arg lib "$lib" --arg package "$package" \
    --arg manifest "$(sha256sum "$dir/Cargo.toml" | cut -d' ' -f1)" \
    --arg empty "$(sha256sum "$dir/$lib" | cut -d' ' -f1)" \
    '{files: {"Cargo.toml": $manifest, ($lib): $empty}, package: $package}' \
    >"$dir/.cargo-checksum.json"
}

# `cargo vendor` ignore la redirection de .cargo/config.toml : il télécharge
# depuis crates.io, vérifie les sommes des Cargo.lock, et retire de vendor/ les
# crates devenus inutiles. Les crates qu'aucune cible livrée ne demande
# (macOS, WebAssembly, Windows GNU ou 32 bits…) sont ensuite vidés.
vendor() {
  local work target dir stubbed=0
  cargo vendor --quiet --locked --versioned-dirs --sync "$fuzz/Cargo.toml" vendor >/dev/null
  work="$(mktemp -d)"
  for target in "${targets[@]}"; do
    needed_by "$root/Cargo.toml" "$target" >>"$work/needed"
  done
  needed_by "$fuzz/Cargo.toml" "${targets[0]}" >>"$work/needed"
  sort -u -o "$work/needed" "$work/needed"
  [[ -s "$work/needed" ]] || { echo "aucun crate demandé : liste des cibles ?" >&2; exit 1; }
  for dir in vendor/*/; do
    dir="${dir%/}"
    if ! grep -qxF -- "${dir#vendor/}" "$work/needed"; then
      stub "$dir"
      stubbed=$((stubbed + 1))
    fi
  done
  rm -rf "$work"
  covered "$root/Cargo.toml" && covered "$fuzz/Cargo.toml"
  echo "vendor/ à jour : $(find vendor -mindepth 1 -maxdepth 1 -type d | wc -l) crates, dont $stubbed vidés (hors cibles livrées), $(du -sh vendor | cut -f1)."
}

case "${1:-}" in
  --check)
    covered "$root/Cargo.toml" && covered "$fuzz/Cargo.toml" || {
      echo "vendor/ ne couvre plus les Cargo.lock : script/vendor.sh" >&2
      exit 1
    }
    echo "vendor/ couvre les Cargo.lock du produit et du fuzzing."
    ;;
  --verify)
    vendor
    changed="$(git status --porcelain --ignored -- vendor)"
    if [[ -n "$changed" ]]; then
      echo "vendor/ diffère de ce que crates.io fournit pour les Cargo.lock :" >&2
      printf '%s\n' "$changed" | head -n 50 >&2
      exit 1
    fi
    echo "vendor/ identique aux sources de crates.io."
    ;;
  --update)
    shift
    # cargo lit .cargo/config.toml depuis le répertoire courant : lancé hors du
    # dépôt, il résout sur crates.io et non sur vendor/.
    (cd / && cargo update --manifest-path "$root/Cargo.toml" "$@")
    "$root/script/fuzz/sync-lock.sh"
    vendor
    ;;
  "")
    vendor
    ;;
  *)
    echo "usage: vendor.sh [--check | --verify | --update [options de cargo update]]" >&2
    exit 2
    ;;
esac
