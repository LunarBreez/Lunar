#!/usr/bin/env bash
# Test du contrôle de chargement différé de wpcap.dll : prouve qu'il accepte
# un exécutable conforme et qu'il REFUSE les autres. Aucun exécutable Windows
# n'est fabriqué ici : llvm-readobj est remplacé par un script qui rend une
# liste d'imports au format réel de `llvm-readobj --coff-imports`. Le test
# porte donc sur la lecture de cette liste, pas sur llvm-readobj lui-même.
# Lancé par le job « Garde-fous de release » de la CI.
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$(cd "$root/.." && pwd)/check-windows-npcap-delayload.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fail() {
  echo "ÉCHEC : $1" >&2
  exit 1
}

# Faux llvm-readobj : rend le fichier `<binaire>.imports`, ou échoue s'il
# n'existe pas.
cat >"$work/readobj" <<'STUB'
#!/usr/bin/env bash
[[ "$1" == "--coff-imports" && -f "$2.imports" ]] || exit 1
cat -- "$2.imports"
STUB
chmod +x "$work/readobj"

accepted() {
  LLVM_READOBJ="$work/readobj" "$script" "$1" >/dev/null 2>&1
}

# Écrit un faux exécutable et sa liste d'imports, lue sur l'entrée standard.
executable() {
  : >"$work/$1"
  cat >"$work/$1.imports"
}

header() {
  printf '\nFile: lunar.exe\nFormat: COFF-x86-64\nArch: x86_64\nAddressSize: 64bit\n'
}

direct() {
  printf 'Import {\n  Name: %s\n  ImportLookupTableRVA: 0x1E064\n' "$1"
  printf '  ImportAddressTableRVA: 0x1E230\n  Symbol: %s (171)\n}\n' "$2"
}

delayed() {
  printf 'DelayImport {\n  Name: %s\n  Attributes: 0x1\n  ModuleHandle: 0x2AF8\n' "$1"
  printf '  ImportAddressTable: 0x2B10\n  ImportNameTable: 0x2390\n'
  printf '  BoundDelayImportTable: 0x0\n  UnloadDelayImportTable: 0x0\n'
  printf '  Import {\n    Symbol: %s (0)\n    Address: 0x140001000\n  }\n}\n' "$2"
}

# --- Cas 1 : wpcap.dll en différé -----------------------------------------
{ header; direct KERNEL32.dll CreateFileW; delayed wpcap.dll pcap_open_live; } |
  executable conforme.exe
accepted "$work/conforme.exe" || fail "wpcap.dll en chargement différé doit être acceptée"

# --- Cas 2 : wpcap.dll en import direct -----------------------------------
# Le cas qui compte : LUNAR ne démarrerait pas sans Npcap.
{ header; direct KERNEL32.dll CreateFileW; direct wpcap.dll pcap_open_live; } |
  executable direct.exe
if accepted "$work/direct.exe"; then
  fail "wpcap.dll importée directement doit être refusée"
fi

# --- Cas 3 : en direct et en différé à la fois ----------------------------
{ header; direct wpcap.dll pcap_lib_version; delayed wpcap.dll pcap_open_live; } |
  executable double.exe
if accepted "$work/double.exe"; then
  fail "wpcap.dll importée directement doit être refusée même si elle l'est aussi en différé"
fi

# --- Cas 4 : casse du nom et retours chariot ------------------------------
# Le nom vient de la bibliothèque d'import (« WPCAP.DLL » possible), et la
# sortie d'un llvm-readobj Windows se termine par CR LF.
{ header; direct KERNEL32.dll CreateFileW; direct WPCAP.DLL pcap_open_live; } |
  sed 's/$/\r/' | executable majuscules.exe
if accepted "$work/majuscules.exe"; then
  fail "WPCAP.DLL importée directement doit être refusée, CR LF compris"
fi
{ header; direct KERNEL32.dll CreateFileW; delayed WPCAP.DLL pcap_open_live; } |
  sed 's/$/\r/' | executable majuscules_differe.exe
accepted "$work/majuscules_differe.exe" ||
  fail "WPCAP.DLL en chargement différé doit être acceptée, CR LF compris"

# --- Cas 5 : Packet.dll en import direct ----------------------------------
{ header; direct Packet.dll PacketGetVersion; delayed wpcap.dll pcap_open_live; } |
  executable packet.exe
if accepted "$work/packet.exe"; then
  fail "Packet.dll importée directement doit être refusée"
fi

# --- Cas 6 : wpcap.dll absente --------------------------------------------
# Un exécutable qui n'importe pas wpcap.dll n'est pas LUNAR.
{ header; direct KERNEL32.dll CreateFileW; } | executable sans_pcap.exe
if accepted "$work/sans_pcap.exe"; then
  fail "un exécutable sans wpcap.dll doit être refusé"
fi

# --- Cas 7 : rien de lisible ----------------------------------------------
header | executable vide.exe
if accepted "$work/vide.exe"; then
  fail "un fichier sans import doit être refusé"
fi
: >"$work/illisible.exe"
if accepted "$work/illisible.exe"; then
  fail "un échec de llvm-readobj doit être refusé"
fi
if accepted "$work/absent.exe"; then
  fail "un binaire introuvable doit être refusé"
fi

echo "Npcap : le contrôle du chargement différé accepte et refuse comme attendu."
