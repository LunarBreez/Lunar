#!/usr/bin/env bash
# Dépendances vendorisées : vendor/ contient les sources de toutes les
# dépendances, aux versions exactes des Cargo.lock du produit et du crate de
# fuzzing, toutes plateformes confondues. Le dossier est commité et
# .cargo/config.toml y redirige crates.io : rien ne se compile depuis le réseau.
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

# `cargo vendor` ignore la redirection de .cargo/config.toml : il télécharge
# depuis crates.io, vérifie les sommes des Cargo.lock, et retire de vendor/ les
# crates devenus inutiles.
vendor() {
  cargo vendor --quiet --locked --versioned-dirs --sync "$fuzz/Cargo.toml" vendor >/dev/null
  covered "$root/Cargo.toml" && covered "$fuzz/Cargo.toml"
  echo "vendor/ à jour : $(find vendor -mindepth 1 -maxdepth 1 -type d | wc -l) crates, $(du -sh vendor | cut -f1)."
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
