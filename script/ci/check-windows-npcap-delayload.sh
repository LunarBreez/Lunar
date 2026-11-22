#!/usr/bin/env bash
# Vérifie que lunar.exe charge wpcap.dll en différé. En import direct, Windows
# refuse de lancer LUNAR quand Npcap est absent ou installé sans le mode
# « WinPcap API-compatible » (0xC0000135, aucune fenêtre) ; en différé,
# lunar_engine::npcap la charge depuis System32\Npcap et LUNAR explique
# l'absence de Npcap. Packet.dll, la couche basse de Npcap, ne doit pas non
# plus figurer dans les imports directs. Le chargement différé est demandé à
# l'éditeur de liens par crates/lunar/build.rs : ce script vérifie le résultat
# sur le binaire, depuis Linux comme depuis Windows.
#
# Usage : check-windows-npcap-delayload.sh <lunar.exe>
#   LLVM_READOBJ désigne llvm-readobj s'il n'est pas dans le PATH ; à défaut,
#   celui du composant rustup « llvm-tools » de la toolchain active est pris.
set -Eeuo pipefail

binary="${1:?usage: check-windows-npcap-delayload.sh <lunar.exe>}"

fail() {
  echo "ÉCHEC : $1" >&2
  exit 1
}

[[ -f "$binary" ]] || fail "binaire introuvable : $binary"

# llvm-readobj distingue les imports directs (« Import { ») des imports
# différés (« DelayImport { ») et ne traduit pas sa sortie.
readobj="${LLVM_READOBJ:-}"
if [[ -z "$readobj" ]]; then
  readobj="$(command -v llvm-readobj || true)"
fi
if [[ -z "$readobj" ]] && command -v rustc >/dev/null 2>&1; then
  sysroot="$(rustc --print sysroot)"
  # Sous Git Bash, rustc rend un chemin Windows que find ne parcourt pas.
  if command -v cygpath >/dev/null 2>&1; then
    sysroot="$(cygpath -u "$sysroot")"
  fi
  readobj="$(find "$sysroot/lib/rustlib" -maxdepth 3 -type f \
    \( -name llvm-readobj -o -name llvm-readobj.exe \) 2>/dev/null | head -n 1)"
fi
if [[ -z "$readobj" ]]; then
  readobj="$(find /usr/lib -maxdepth 3 -path '/usr/lib/llvm-*/bin/llvm-readobj' 2>/dev/null |
    sort -V | tail -n 1)"
fi
[[ -n "$readobj" ]] ||
  fail "llvm-readobj est requis (rustup component add llvm-tools, ou LLVM_READOBJ)"

# Sortie lue une fois dans une variable : un échec de llvm-readobj arrête le
# script au lieu de passer pour « aucun import ».
imports="$("$readobj" --coff-imports "$binary")" ||
  fail "imports illisibles : $binary"

# Une ligne « direct nom.dll » ou « différé nom.dll » par bibliothèque : le nom
# suit l'ouverture de son bloc. Minuscules, sans retour chariot (Windows).
listing="$(tr -d '\r' <<<"$imports" | awk '
  /^[[:space:]]*Import \{/      { section = "direct" }
  /^[[:space:]]*DelayImport \{/ { section = "différé" }
  /^[[:space:]]*Name: / && section != "" { print section, tolower($2); section = "" }
')"
grep -q '^direct ' <<<"$listing" ||
  fail "aucun import direct lu dans $binary : ce n'est pas un exécutable Windows"

for library in wpcap.dll packet.dll; do
  if grep -qx "direct $library" <<<"$listing"; then
    fail "$library est importée directement par $binary (chargement différé attendu)"
  fi
done
grep -qx 'différé wpcap.dll' <<<"$listing" ||
  fail "wpcap.dll absente des imports différés de $binary"
echo "Npcap : wpcap.dll chargée en différé par $binary."
