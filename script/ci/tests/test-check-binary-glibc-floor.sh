#!/usr/bin/env bash
# Test du plancher glibc : prouve que le contrôle accepte un binaire sous le
# plancher et qu'il REFUSE un binaire qui le dépasse. Le binaire d'essai est
# compilé sur place : sa version maximale est celle des symboles que la glibc
# de la machine lui donne, lue ici indépendamment du script éprouvé. Lancé par
# le job « Garde-fous de release » de la CI.
#
# REQUIRE_CC=1 interdit d'ignorer le test faute de compilateur C (CI).
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$(cd "$root/.." && pwd)/check-binary-glibc-floor.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cc="${CC:-cc}"
if ! command -v "$cc" >/dev/null 2>&1; then
  if [[ "${REQUIRE_CC:-0}" == "1" ]]; then
    echo "ÉCHEC : aucun compilateur C, le contrôle ne peut pas être éprouvé" >&2
    exit 1
  fi
  echo "SKIP : aucun compilateur C, le contrôle ne peut pas être éprouvé" >&2
  exit 0
fi

fail() {
  echo "ÉCHEC : $1" >&2
  exit 1
}

accepted() {
  "$script" "$@" >/dev/null 2>&1
}

cat >"$work/hello.c" <<'SOURCE'
#include <stdio.h>
int main(void) { return puts("lunar") < 0; }
SOURCE
"$cc" -o "$work/hello" "$work/hello.c"

# Version maximale exigée par le binaire d'essai, lue sans le script.
max="$(objdump -T "$work/hello" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' |
  sed 's/^GLIBC_//' | sort -t. -k1,1n -k2,2n | tail -n 1)"
[[ "$max" =~ ^[0-9]+\.[0-9]+$ ]] || fail "version glibc du binaire d'essai illisible"
major="${max%%.*}"
minor="${max#*.}"

# --- Cas 1 : plancher égal à la version exigée ----------------------------
accepted "$work/hello" "$max" || fail "un plancher égal à la version exigée doit être accepté"

# --- Cas 2 : plancher au-dessus --------------------------------------------
accepted "$work/hello" "$major.$((10#$minor + 1))" ||
  fail "un plancher supérieur à la version exigée doit être accepté"
accepted "$work/hello" "$((10#$major + 1)).0" ||
  fail "un plancher de majeur supérieur doit être accepté"

# --- Cas 3 : plancher en dessous -------------------------------------------
# Le cas qui compte : le binaire exige plus que ce que la cible fournit.
if ((10#$minor > 0)); then
  below="$major.$((10#$minor - 1))"
else
  below="$((10#$major - 1)).99"
fi
if accepted "$work/hello" "$below"; then
  fail "un binaire exigeant GLIBC_$max doit être refusé sous le plancher $below"
fi

# --- Cas 4 : comparaison numérique, pas alphabétique -----------------------
# « 2.9 » est plus ancien que « 2.35 » : un tri de chaînes dirait l'inverse.
if ((10#$major == 2 && 10#$minor >= 10)); then
  if accepted "$work/hello" "2.9"; then
    fail "le plancher 2.9 doit refuser un binaire exigeant GLIBC_$max"
  fi
fi

# --- Cas 5 : entrées invalides ---------------------------------------------
if accepted "$work/absent" "$max"; then
  fail "un binaire introuvable doit être refusé"
fi
if accepted "$work/hello" "deux.trente"; then
  fail "un plancher non numérique doit être refusé"
fi
if accepted "$work/hello.c" "$max"; then
  fail "un fichier qui n'est pas un binaire doit être refusé"
fi

# --- Cas 6 : binaire statique ----------------------------------------------
# Sans symbole dynamique, rien n'est vérifiable : le contrôle refuse plutôt
# que de conclure. Ignoré si la machine n'a pas la libc statique.
if "$cc" -static -o "$work/static" "$work/hello.c" >/dev/null 2>&1; then
  if accepted "$work/static" "$max"; then
    fail "un binaire statique doit être refusé, faute de version à lire"
  fi
fi

echo "Plancher glibc : le contrôle accepte et refuse comme attendu (binaire d'essai GLIBC_$max)."
