#!/usr/bin/env bash
# Test du contrôle de passivité : prouve qu'il accepte un binaire muet et
# qu'il REFUSE un binaire capable d'émettre. Un contrôle qu'on n'a jamais vu
# échouer ne prouve rien. Lancé par le job « Garde-fous de release » de la CI.
#
# REQUIRE_LIBPCAP=1 interdit d'ignorer le cas pcap_sendpacket quand libpcap
# manque (CI).
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$(cd "$root/.." && pwd)/check-binary-is-passive.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cc="${CC:-cc}"
if ! command -v "$cc" >/dev/null 2>&1; then
  if [[ "${REQUIRE_LIBPCAP:-0}" == "1" ]]; then
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
  "$script" "$1" >/dev/null 2>&1
}

# --- Cas 1 : binaire strictement passif -----------------------------------
cat >"$work/passive.c" <<'EOF'
int main(void) { return 0; }
EOF
"$cc" -o "$work/passive" "$work/passive.c"
"$script" "$work/passive" >/dev/null ||
  fail "un binaire sans primitive d'émission doit être accepté"

# --- Cas 2 : les sockets restent autorisés --------------------------------
# Le serveur X11 et les portails du bureau se joignent par des sockets :
# interdire `sendto` reviendrait à interdire l'affichage. Ce cas verrouille
# cette décision — durcir le contrôle casserait le binaire réel, pas
# seulement ce test.
cat >"$work/socket.c" <<'EOF'
#include <sys/socket.h>
#include <stddef.h>

int main(void) {
  volatile int run = 0;
  if (run) {
    sendto(0, NULL, 0, 0, NULL, 0);
  }
  return 0;
}
EOF
"$cc" -o "$work/socket" "$work/socket.c"
accepted "$work/socket" || fail "les sockets doivent rester autorisés"

# --- Cas 3 : client HTTP embarqué -----------------------------------------
cat >"$work/http.c" <<'EOF'
const char *marker = "reqwest::Client";
int main(void) { return marker[0]; }
EOF
"$cc" -o "$work/http" "$work/http.c"
if accepted "$work/http"; then
  fail "un binaire embarquant un client HTTP doit être refusé"
fi

# --- Cas 4 : client HTTP dans un gros binaire -----------------------------
# La chaîne interdite arrive en tête, suivie de plusieurs mégaoctets de
# chaînes. Une recherche écrite `strings … | grep -q` sous `pipefail`
# accepterait ce binaire : grep sort à la première correspondance, strings
# reçoit SIGPIPE, le tube rend 141 et la condition passe pour fausse. Le
# binaire réel pèse des dizaines de mégaoctets : c'est ce cas qui lui
# ressemble, pas le précédent.
{
  printf 'const char blob[] = "reqwest::Client\\n"\n'
  awk 'BEGIN {
    for (i = 0; i < 40000; i++)
      print "\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\\n\""
  }'
  printf ';\nint main(void) { return blob[0]; }\n'
} >"$work/big_http.c"
"$cc" -o "$work/big_http" "$work/big_http.c"
if accepted "$work/big_http"; then
  fail "un client HTTP doit être refusé même en tête d'un gros binaire"
fi

# --- Cas 5 : nom de symbole seul ------------------------------------------
# Le binaire livré garde sa table des symboles, où « zed_reqwest » figure
# sans qu'aucun client soit lié (code de libération de types empruntés à
# reqwest par gpui_http_client). Un nom de symbole n'est pas du code : le
# contrôle porte sur ce qui reste après `strip`.
cat >"$work/symbol_only.c" <<'EOF'
void zed_reqwest_drop_glue(void) {}
int main(void) {
  zed_reqwest_drop_glue();
  return 0;
}
EOF
"$cc" -o "$work/symbol_only" "$work/symbol_only.c"
accepted "$work/symbol_only" ||
  fail "un nom de symbole sans donnée ni code de client HTTP doit être accepté"

# --- Cas 6 : binaire déjà strippé -----------------------------------------
strip -o "$work/passive_stripped" "$work/passive"
accepted "$work/passive_stripped" || fail "un binaire déjà strippé doit être accepté"
strip -o "$work/http_stripped" "$work/http"
if accepted "$work/http_stripped"; then
  fail "un client HTTP doit être refusé dans un binaire strippé"
fi

# --- Cas 7 : binaire appelant pcap_sendpacket -----------------------------
# Le cas qui compte vraiment pour LUNAR. En local, ignoré si libpcap n'est
# pas installée ; en CI, REQUIRE_LIBPCAP=1 interdit ce silence.
if "$cc" -o "$work/injector" -xc - -lpcap >/dev/null 2>&1 <<'EOF'
#include <pcap.h>
#include <stddef.h>

int main(void) {
  volatile int run = 0;
  if (run) {
    pcap_sendpacket(NULL, NULL, 0);
  }
  return 0;
}
EOF
then
  if accepted "$work/injector"; then
    fail "un binaire important pcap_sendpacket doit être refusé"
  fi
elif [[ "${REQUIRE_LIBPCAP:-0}" == "1" ]]; then
  fail "REQUIRE_LIBPCAP=1 mais libpcap indisponible : le cas pcap_sendpacket n'a pas été éprouvé"
else
  echo "Note : libpcap absente, cas pcap_sendpacket ignoré." >&2
fi

# --- Cas 8 : binaire passif lié à libpcap ---------------------------------
# Pendant du cas 7 : lire une capture n'est pas émettre. EXPECT_LIBPCAP=1
# doit l'accepter, comme l'artefact de release.
if "$cc" -o "$work/reader" -xc - -lpcap >/dev/null 2>&1 <<'EOF'
#include <pcap.h>
#include <stddef.h>

int main(void) {
  volatile int run = 0;
  if (run) {
    pcap_close(NULL);
  }
  return 0;
}
EOF
then
  EXPECT_LIBPCAP=1 "$script" "$work/reader" >/dev/null 2>&1 ||
    fail "un binaire qui lit via libpcap sans émettre doit être accepté"
elif [[ "${REQUIRE_LIBPCAP:-0}" == "1" ]]; then
  fail "REQUIRE_LIBPCAP=1 mais libpcap indisponible : le cas du lecteur libpcap n'a pas été éprouvé"
fi

# --- Cas 9 : binaire absent -----------------------------------------------
if accepted "$work/inexistant"; then
  fail "un chemin inexistant doit être refusé, pas ignoré"
fi

# --- Cas 10 : fichier non-ELF ---------------------------------------------
# `nm` échoue sur un fichier qui n'est pas un objet : le contrôle doit
# propager cet échec, pas conclure « aucun symbole importé, donc passif ».
printf 'pas un ELF\n' >"$work/not_elf"
if accepted "$work/not_elf"; then
  fail "un fichier non-ELF doit être refusé : nm en échec ne vaut pas preuve"
fi

# --- Cas 11 : binaire statique --------------------------------------------
# Sans table d'imports dynamiques, le contrôle par symboles ne voit plus
# rien : il doit refuser plutôt que valider dans le vide. Ignoré si la libc
# statique n'est pas installée.
if "$cc" -static -o "$work/static_bin" "$work/passive.c" >/dev/null 2>&1; then
  if accepted "$work/static_bin"; then
    fail "un binaire statique doit être refusé : aucune table d'imports à vérifier"
  fi
else
  echo "Note : libc statique absente, cas binaire statique ignoré." >&2
fi

# --- Cas 12 : EXPECT_LIBPCAP=1 sur un binaire sans libpcap ----------------
# La release exige que le binaire vérifié importe bien libpcap ; sinon le
# contrôle a été pointé sur le mauvais fichier.
if EXPECT_LIBPCAP=1 "$script" "$work/passive" >/dev/null 2>&1; then
  fail "EXPECT_LIBPCAP=1 doit refuser un binaire qui n'importe aucun symbole pcap_*"
fi

echo "Contrôle de passivité : accepte le binaire muet, refuse l'émetteur et le client HTTP."
