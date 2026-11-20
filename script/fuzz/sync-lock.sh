#!/usr/bin/env bash
# Aligne le Cargo.lock du crate de fuzzing sur celui de la racine : les crates
# communs aux deux y ont la même version, donc le fuzzing porte sur le code
# réellement livré.
#
# Sans option : régénère crates/lunar-flows-core/fuzz/Cargo.lock à partir du
# lock de la racine (cargo retire ce qui ne sert pas au fuzzing et résout les
# crates qui lui sont propres, libfuzzer-sys et arbitrary). À relancer après
# toute mise à jour des dépendances de lunar-flows-core.
# Avec --check : vérifie sans rien écrire (CI).
set -Eeuo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fuzz="$root/crates/lunar-flows-core/fuzz"

# Paires « nom version » des paquets d'un Cargo.lock.
versions() {
  awk '
    /^\[\[package\]\]/ { in_package = 1; next }
    /^\[/              { in_package = 0 }
    in_package && /^name = /    { gsub(/"/, "", $3); name = $3 }
    in_package && /^version = / { gsub(/"/, "", $3); print name, $3 }
  ' "$1" | sort -u
}

# Paquets du lock de fuzzing que la racine connaît sous une autre version.
# Les deux listes passent par des fichiers : un lock absent ou illisible
# arrête le script, au lieu de donner une comparaison vide, donc « alignée ».
drift() {
  local work status=0
  work="$(mktemp -d)"
  versions "$root/Cargo.lock" >"$work/root" && versions "$fuzz/Cargo.lock" >"$work/fuzz" &&
    [[ -s "$work/root" && -s "$work/fuzz" ]] || status=1
  if ((status == 0)); then
    awk 'NR == FNR { known[$1] = 1; pair[$0] = 1; next }
         ($1 in known) && !($0 in pair)' "$work/root" "$work/fuzz" || status=$?
  fi
  rm -rf "$work"
  if ((status != 0)); then
    echo "lecture des Cargo.lock impossible" >&2
    return 1
  fi
}

case "${1:-}" in
  --check)
    # Le lock doit couvrir le manifeste du crate tel quel…
    (cd "$fuzz" && cargo metadata --locked --format-version 1 >/dev/null) || {
      echo "Cargo.lock du fuzzing à régénérer : script/fuzz/sync-lock.sh" >&2
      exit 1
    }
    # … et chaque crate commun avoir la version du produit.
    drifted="$(drift)"
    if [[ -n "$drifted" ]]; then
      echo "Cargo.lock du fuzzing désaligné de celui de la racine :" >&2
      printf '%s\n' "$drifted" | sed 's/^/  /' >&2
      echo "Régénérer : script/fuzz/sync-lock.sh" >&2
      exit 1
    fi
    echo "Cargo.lock du fuzzing aligné sur celui de la racine."
    ;;
  "")
    cp -- "$root/Cargo.lock" "$fuzz/Cargo.lock"
    (cd "$fuzz" && cargo metadata --format-version 1 >/dev/null)
    drifted="$(drift)"
    [[ -z "$drifted" ]] || { echo "alignement impossible : $drifted" >&2; exit 1; }
    echo "Cargo.lock du fuzzing régénéré : $(versions "$fuzz/Cargo.lock" | wc -l) paquets."
    ;;
  *)
    echo "usage: sync-lock.sh [--check]" >&2
    exit 2
    ;;
esac
