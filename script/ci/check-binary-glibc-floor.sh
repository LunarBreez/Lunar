#!/usr/bin/env bash
# Plancher de compatibilité glibc d'un binaire ELF. LUNAR vise Ubuntu 22.04
# (glibc 2.35) : un binaire construit sur un système plus récent peut exiger
# une version de symbole que la cible n'a pas, et le chargeur refuse alors de
# le lancer (« version `GLIBC_2.39' not found »). Ce script lit les versions
# GLIBC_x.y importées par le binaire et refuse toute version au-delà du
# plancher, y compris pour un symbole faible : le chargeur refuse la version
# manquante avant de regarder le symbole.
#
# Usage : check-binary-glibc-floor.sh <binaire> <plancher, ex. 2.35>
set -Eeuo pipefail

binary="${1:?usage: check-binary-glibc-floor.sh <binaire> <plancher>}"
floor="${2:?usage: check-binary-glibc-floor.sh <binaire> <plancher>}"

fail() {
  echo "ÉCHEC : $1" >&2
  exit 1
}

[[ -f "$binary" ]] || fail "binaire introuvable : $binary"
command -v objdump >/dev/null 2>&1 || fail "objdump est requis (binutils)"
[[ "$floor" =~ ^[0-9]+\.[0-9]+$ ]] || fail "plancher invalide : $floor"

# Table des symboles dynamiques, lue une fois dans une variable : un échec
# d'objdump arrête le script au lieu de passer pour « aucune version ».
symbols="$(objdump -T "$binary" 2>/dev/null)" ||
  fail "table des symboles dynamiques illisible : $binary"
versions="$(grep -oE 'GLIBC_[0-9]+\.[0-9]+' <<<"$symbols" | sort -u || true)"
[[ -n "$versions" ]] ||
  fail "aucune version GLIBC lue dans $binary (binaire statique ?)"

# Vrai si $1 > $2, comparés comme des couples majeur.mineur.
newer_than() {
  local a_major="${1%%.*}" a_minor="${1#*.}" b_major="${2%%.*}" b_minor="${2#*.}"
  ((10#$a_major > 10#$b_major || (10#$a_major == 10#$b_major && 10#$a_minor > 10#$b_minor)))
}

max="0.0"
while IFS= read -r version; do
  version="${version#GLIBC_}"
  if newer_than "$version" "$max"; then
    max="$version"
  fi
done <<<"$versions"

if newer_than "$max" "$floor"; then
  echo "ÉCHEC : $binary exige GLIBC_$max, au-delà du plancher $floor. Symboles en cause :" >&2
  awk -v wanted="GLIBC_$max" '
    { for (i = 1; i <= NF; i++) if ($i == wanted || $i == "(" wanted ")") print "  " $NF }
  ' <<<"$symbols" | sort -u >&2
  exit 1
fi
echo "Plancher glibc : $binary n'exige que GLIBC_$max (plancher $floor)."
