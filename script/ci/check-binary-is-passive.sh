#!/usr/bin/env bash
# Preuve de passivité du binaire natif : LUNAR écoute le réseau, il n'y émet
# pas. Ce script en fait une propriété vérifiée sur les octets livrés, à
# chaque release, plutôt qu'une affaire de relecture.
#
# Deux contrôles :
#
#   1. Aucune primitive d'émission libpcap n'est liée. `pcap_sendpacket` et
#      `pcap_inject` sont les deux seules façons d'émettre une trame via
#      libpcap ; si l'éditeur de liens ne les a jamais résolues, aucun chemin
#      de code ne peut les appeler.
#   2. Aucun client HTTP embarqué. `zed-reqwest`, `hyper` et `rustls` figurent
#      dans Cargo.lock, tirés par gpui (gpui_http_client) pour trois types de
#      sa signature ; LUNAR n'installe aucun client, et l'éditeur de liens
#      n'en garde aucun code. Ce contrôle vérifie le binaire, pas le lock.
#
# Ce que le script NE prouve PAS, et qu'il ne faut pas lui faire dire : il
# n'établit pas qu'aucun octet ne sort d'un socket. Le binaire importe
# `socket`, `connect` et `sendto` : le serveur X11 (x11rb) et les portails
# du bureau (zbus) se joignent par des sockets locaux. Cette garantie-là
# repose sur la revue du code appelant, pas sur la table des symboles.
#
# Usage : check-binary-is-passive.sh <binaire>
#   EXPECT_LIBPCAP=1 exige en plus que le binaire importe libpcap (artefact
#   de release : sans cela, ce n'est pas le bon fichier).
set -Eeuo pipefail

binary="${1:?usage: check-binary-is-passive.sh <binaire>}"

fail() {
  echo "ÉCHEC : $1" >&2
  exit 1
}

[[ -f "$binary" ]] || fail "binaire introuvable : $binary"

# ELF et binutils GNU seulement. Le binaire Windows demanderait un autre
# outillage (table d'imports PE) : plutôt que d'expédier une variante jamais
# éprouvée, le contrôle s'annonce non couvert. La passivité n'est donc prouvée
# que pour l'artefact Linux.
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Passivité : contrôle non couvert sur $(uname -s), ignoré." >&2
  exit 0
fi

# Un outil absent ne vaut jamais preuve : le contrôle échoue au lieu de se
# réduire en silence.
for tool in nm strip strings; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool est requis (binutils)"
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Vrai si le fichier contient la ligne ou la sous-chaîne cherchée. `grep`
# lit un fichier, jamais un tube : sous `pipefail`, un `producteur | grep -q`
# rend 141 (SIGPIPE du producteur) dès que la correspondance arrive tôt dans
# une grosse sortie, et la condition passerait pour fausse — un contrôle qui
# se tait précisément quand il trouve. Une erreur de grep arrête le script.
file_has() {
  local mode="$1" needle="$2" file="$3" status=0
  grep "$mode" -q -- "$needle" "$file" || status=$?
  case "$status" in
    0) return 0 ;;
    1) return 1 ;;
    *) fail "grep a échoué sur $file (code $status)" ;;
  esac
}

# --- 1. Primitives d'émission ----------------------------------------------
# `nm -D` liste les symboles dynamiques ; `--undefined-only` garde ceux que le
# binaire importe d'une bibliothèque partagée. Un symbole d'émission ne peut
# être appelé que s'il y figure.
#
# Un échec de `nm` ne vaut pas preuve : sur un fichier non-ELF (artefact
# corrompu, mauvais chemin), « aucun symbole importé » signifierait « aucune
# vérification faite ». stderr reste hors de la capture : sur un binaire
# statique, nm sort 0 en écrivant « no symbols » sur stderr.
if ! nm -D --undefined-only "$binary" >"$work/nm" 2>/dev/null; then
  nm -D --undefined-only "$binary" >/dev/null || true
  fail "nm ne peut pas lire la table dynamique de $binary"
fi
# Dernier champ de chaque ligne, sans le suffixe de version : la glibc exporte
# `sendto@GLIBC_2.2.5`, pas `sendto`.
awk '{ print $NF }' "$work/nm" | sed 's/@.*$//' >"$work/imports"
# Table d'imports vide : binaire statique, dont les liens ne passent par
# aucune table que ce contrôle sait lire. Refusé plutôt que validé dans le
# vide.
[[ -s "$work/imports" ]] ||
  fail "aucune table d'imports dynamiques dans $binary (binaire statique ?)"

forbidden_symbols=(
  pcap_sendpacket
  pcap_inject
)
found_symbols=()
for symbol in "${forbidden_symbols[@]}"; do
  if file_has -Fx "$symbol" "$work/imports"; then
    found_symbols+=("$symbol")
  fi
done
if ((${#found_symbols[@]} > 0)); then
  fail "le binaire importe des primitives d'émission : ${found_symbols[*]}"
fi

# --- 2. Client HTTP embarqué ------------------------------------------------
# Un client HTTP laisse des chaînes reconnaissables dans le code et les
# données : noms de crate dans les messages de panique, les chemins de
# sources et les noms de types.
#
# La recherche porte sur une copie sans table des symboles. Le binaire livré
# garde la sienne, où « zed_reqwest » apparaît sans qu'aucun client soit lié :
# noms d'unités de compilation, et code de libération des trois types que
# gpui_http_client emprunte à reqwest. Un nom dans la table des symboles n'est
# pas du code ; ce qui reste après `strip` est ce que le chargeur projette en
# mémoire.
strip -o "$work/stripped" "$binary" 2>/dev/null ||
  fail "strip ne peut pas traiter $binary"
strings -a "$work/stripped" >"$work/strings"
[[ -s "$work/strings" ]] || fail "aucune chaîne extraite de $binary"

forbidden_strings=(
  reqwest
  hyper_util
  rustls
)
found_strings=()
for needle in "${forbidden_strings[@]}"; do
  if file_has -F "$needle" "$work/strings"; then
    found_strings+=("$needle")
  fi
done
if ((${#found_strings[@]} > 0)); then
  fail "le binaire embarque un client HTTP : ${found_strings[*]}"
fi

# --- Garde-fou contre le contrôle à vide ------------------------------------
# Le binaire LUNAR réel importe une vingtaine de symboles pcap_* (findalldevs,
# next_ex…). S'il n'en importe aucun, ce n'est pas le bon binaire, ou libpcap
# est passée en lien statique et le contrôle par symboles ne voit plus rien.
pcap_symbols="$(grep -c '^pcap_' "$work/imports" || true)"
if [[ "${EXPECT_LIBPCAP:-0}" == "1" && "$pcap_symbols" -eq 0 ]]; then
  fail "EXPECT_LIBPCAP=1 mais aucun symbole pcap_* importé (mauvais binaire, ou libpcap liée statiquement)"
fi

echo "Passivité vérifiée : $pcap_symbols symboles libpcap importés, aucun d'émission ; aucun client HTTP."
