#!/usr/bin/env bash
# Parcours E2E du binaire release de LUNAR sous X11.
#
# Le vrai binaire `lunar` est piloté au clavier et à la souris (XTest)
# sur un écran Xvfb qui lui est propre, dans un espace de noms utilisateur +
# réseau (`unshare -rn`) où seule l'interface `lo` existe : toute la session
# se déroule sans accès au réseau, et la capture live (--live-capture) y
# dispose de CAP_NET_RAW sans sudo. Les preuves sont les journaux de
# l'application, les fichiers exportés et une capture d'écran par étape.
#
# Le script n'arrête que les processus qu'il a lancés (PID mémorisés), ne
# touche ni à l'écran ni aux données de l'utilisateur (XDG_* et HOME isolés
# dans le dossier d'artefacts) et refuse un écran déjà occupé.
#
# Voir script/e2e/README.md.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_PATH="$ROOT_DIR/script/e2e/$(basename "${BASH_SOURCE[0]}")"

binary_path="$ROOT_DIR/target/release/lunar"
parity_path="$ROOT_DIR/target/release/examples/parity_csv"
artifacts_dir=""
display_name=":121"
live_capture=0
live_seconds=5
build_binary=0

usage() {
  cat <<'EOF'
Usage: run-lunar-x11-e2e.sh [options]

Options:
  --binary PATH       Binaire lunar à tester
                      (défaut : target/release/lunar).
  --parity PATH       Exemple parity_csv de référence pour l'export CSV
                      (défaut : target/release/examples/parity_csv).
  --artifacts DIR     Dossier des preuves ; doit être absent ou vide
                      (défaut : dossier temporaire).
  --display :N        Écran Xvfb dédié, refusé s'il est occupé (défaut : :121).
  --live-capture      Teste aussi la capture live sur lo (filtre icmp, ping).
  --live-seconds N    Durée du trafic de la capture live (défaut : 5).
  --build             Compile le binaire et parity_csv en release avant.
  -h, --help          Affiche cette aide.
EOF
}

while (($# > 0)); do
  case "$1" in
    --binary) binary_path="${2:?--binary attend un chemin}"; shift 2 ;;
    --parity) parity_path="${2:?--parity attend un chemin}"; shift 2 ;;
    --artifacts) artifacts_dir="${2:?--artifacts attend un chemin}"; shift 2 ;;
    --display) display_name="${2:?--display attend :N}"; shift 2 ;;
    --live-capture) live_capture=1; shift ;;
    --live-seconds) live_seconds="${2:?--live-seconds attend un entier}"; shift 2 ;;
    --build) build_binary=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Option inconnue : $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$display_name" =~ ^:[1-9][0-9]*$ ]] || {
  echo "--display attend :N (N > 0) : $display_name" >&2
  exit 2
}
[[ "$live_seconds" =~ ^[1-9][0-9]*$ ]] || {
  echo "--live-seconds attend un entier strictement positif" >&2
  exit 2
}

checks_file=""
pass_count=0
current_step="amorçage"

log() {
  printf '[LUNAR E2E] %s\n' "$*"
}

pass() {
  pass_count=$((pass_count + 1))
  printf 'PASS %2d  %s\n' "$pass_count" "$*" >>"$checks_file"
  log "PASS $pass_count — $*"
}

fail() {
  [[ -z "$checks_file" ]] || printf 'FAIL     %s — %s\n' "$current_step" "$*" >>"$checks_file"
  log "FAIL — $current_step — $*" >&2
  exit 1
}

step() {
  current_step="$*"
  log "Étape : $*"
}

# shellcheck source-path=SCRIPTDIR source=lunar-x11-lib.sh
source "$ROOT_DIR/script/e2e/lunar-x11-lib.sh"

# --- Amorçage : hôte → espace de noms → session ------------------------------

if [[ "${LUNAR_E2E_STAGE:-host}" == "host" ]]; then
  display_is_free || {
    echo "l'écran $display_name est occupé : choisir --display" >&2
    exit 1
  }
  if ((build_binary == 1)); then
    (cd "$ROOT_DIR" &&
      cargo build --release --locked -p lunar --bin lunar &&
      cargo build --release --locked -p lunar-engine --example parity_csv)
  fi
  prepare_artifacts_dir lunar-e2e
  binary_path="$(absolute_path "$binary_path")"
  parity_path="$(absolute_path "$parity_path")"
fi
inner=(--binary "$binary_path" --parity "$parity_path"
  --artifacts "$artifacts_dir" --display "$display_name"
  --live-seconds "$live_seconds")
if ((live_capture == 1)); then inner+=(--live-capture); fi
enter_isolation "$SCRIPT_PATH" "${inner[@]}"

# --- Session de test --------------------------------------------------------

summary_file="$artifacts_dir/summary.txt"
checks_file="$artifacts_dir/checks.txt"
fixtures_dir="$ROOT_DIR/fixtures"
sampler_pid=""
: >"$checks_file"

cleanup() {
  local status="$?"
  set +e
  if ((status != 0)) && [[ -n "$main_id" ]] && [[ -n "$app_pid" ]] &&
    kill -0 "$app_pid" 2>/dev/null; then
    capture_window "echec" >/dev/null 2>&1
  fi
  stop_pid "$sampler_pid"
  stop_pid "$app_pid"
  stop_pid "$xvfb_pid"
  # Journaux de l'application tels qu'écrits sur disque.
  if [[ -d "${XDG_DATA_HOME:-}/fr.lunar/logs" ]]; then
    cp -r "$XDG_DATA_HOME/fr.lunar/logs" "$artifacts_dir/app-logs" 2>/dev/null
  fi
  {
    printf 'Binaire : %s\n' "$binary_path"
    printf 'Écran : %s (Xvfb dédié), réseau : espace de noms isolé (lo seule)\n' "$display_name"
    printf 'Capture live : %s\n' "$([[ $live_capture == 1 ]] && echo oui || echo non)"
    printf 'PCAP : %s\n' "${pcap_fixture:-}"
    printf 'Matrice : %s\n' "${matrix_fixture:-}"
    printf 'Labels : %s\n' "${labels_fixture:-}"
    printf 'Relevé WinAudit : %s\n' "${winaudit_fixture:-}"
    printf '\n'
    cat "$checks_file"
    printf '\nContrôles réussis : %d\n' "$pass_count"
    if ((status == 0)); then
      printf 'LUNAR X11 E2E: PASS\n'
    else
      printf 'Échec à l étape : %s (code %d)\n' "$current_step" "$status"
      printf 'LUNAR X11 E2E: FAIL\n'
    fi
  } >"$summary_file"
  cat "$summary_file"
  log "Artefacts : $artifacts_dir"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "commande requise absente : $1"
}

# --- Réseau : aucune socket hors boucle locale ------------------------------

# Sockets de LUNAR : une ligne « proto local distant » par socket inet
# (tcp, udp, raw ; v4 et v6 ; adresses en hexadécimal du noyau) et
# « packet ifindex:N - » par socket de capture (AF_PACKET, interface N, 0 =
# toutes : ici `lo` seule).
app_inet_sockets() {
  local pid="$1" inodes proto file
  inodes="$(find "/proc/$pid/fd" -maxdepth 1 -type l -printf '%l\n' 2>/dev/null |
    sed -n 's/^socket:\[\([0-9]*\)\]$/\1/p' | sort -u | tr '\n' ' ')"
  [[ -n "$inodes" ]] || return 0
  for proto in tcp tcp6 udp udp6 raw raw6; do
    file="/proc/$pid/net/$proto"
    [[ -r "$file" ]] || continue
    awk -v proto="$proto" -v inodes=" $inodes" '
      NR > 1 && index(inodes, " " $10 " ") { print proto, $2, $3 }' "$file"
  done
  if [[ -r "/proc/$pid/net/packet" ]]; then
    awk -v inodes=" $inodes" '
      NR > 1 && index(inodes, " " $9 " ") { print "packet", "ifindex:" $5, "-" }' \
      "/proc/$pid/net/packet"
  fi
}

# Vrai si l'adresse hexadécimale (avec :port) est une boucle locale ou nulle,
# ou si la socket de capture vise `lo` (index 1) ou toutes les interfaces de
# l'espace de noms (0, qui ne contient que `lo`).
is_loopback_hex() {
  local addr="${1%:*}"
  case "$1" in
    ifindex:0|ifindex:1|-) return 0 ;;
  esac
  case "$addr" in
    00000000|0100007F|??????7F) return 0 ;;
    00000000000000000000000000000000|00000000000000000000000001000000) return 0 ;;
    0000000000000000FFFF0000??????7F) return 0 ;;
  esac
  return 1
}

# Relève périodique des sockets de l'application (toute la session).
network_sampler() {
  local pid="$1" out="$2"
  while kill -0 "$pid" 2>/dev/null; do
    app_inet_sockets "$pid" | sed "s/^/$(date +%s) /" >>"$out"
    sleep 1
  done
}

assert_no_network() {
  local label="$1" proto local_addr remote_addr bad=0
  ip -o link show >"$artifacts_dir/reseau-$label-interfaces.txt"
  if grep -vq ': lo:' "$artifacts_dir/reseau-$label-interfaces.txt"; then
    fail "interface autre que lo dans l'espace de noms ($label)"
  fi
  ss -tuwanp >"$artifacts_dir/reseau-$label-ss.txt" 2>&1 || true
  while read -r proto local_addr remote_addr; do
    [[ -n "$proto" ]] || continue
    if ! is_loopback_hex "$local_addr" || ! is_loopback_hex "$remote_addr"; then
      log "socket hors boucle locale : $proto $local_addr -> $remote_addr"
      bad=1
    fi
  done < <(app_inet_sockets "$app_pid")
  ((bad == 0)) || fail "LUNAR a ouvert une socket réseau hors boucle locale ($label)"
}

# --- Prérequis --------------------------------------------------------------

step "prérequis"
for command_name in cc pkg-config Xvfb xdpyinfo import identify convert file unzip \
  sha256sum cmp ss ip ping python3 timeout; do
  require_command "$command_name"
done
[[ -x "$binary_path" ]] || fail "binaire introuvable ou non exécutable : $binary_path (--build)"
[[ -x "$parity_path" ]] || fail "parity_csv introuvable : $parity_path (--build)"

pcap_fixture="${LUNAR_E2E_PCAP:-$fixtures_dir/pcaps/import/pcap_tshark_corpus/industrial_ethernet.pcap}"
matrix_fixture="${LUNAR_E2E_MATRIX:-$fixtures_dir/20260703_NP_Matrice.csv}"
labels_fixture="${LUNAR_E2E_LABELS:-$fixtures_dir/20260703_NP_Labels.csv}"
winaudit_fixture="${LUNAR_E2E_WINAUDIT:-$fixtures_dir/winaudit/20260924_NP_CONF_TST-PC-000001.csv}"
for fixture in "$pcap_fixture" "$matrix_fixture" "$labels_fixture" "$winaudit_fixture"; do
  [[ -f "$fixture" ]] || fail "fixture absente : $fixture"
done

compile_input_driver "$ROOT_DIR"
pass "pilote X11 compilé"

step "réseau isolé"
ip -o link show >"$artifacts_dir/reseau-interfaces.txt"
if grep -vq ': lo:' "$artifacts_dir/reseau-interfaces.txt"; then
  fail "l'espace de noms réseau contient une autre interface que lo"
fi
grep -q 'LOOPBACK,UP' "$artifacts_dir/reseau-interfaces.txt" ||
  fail "lo n'est pas levée dans l'espace de noms"
pass "espace de noms réseau isolé : lo seule ($(id -u) = root de l'espace de noms)"

# --- Écran et environnement isolés ------------------------------------------

step "écran Xvfb"
start_xvfb
pass "écran Xvfb dédié prêt sur $DISPLAY (Composite désactivé)"
export_isolated_env

# --- 1. Smoke test de démarrage ---------------------------------------------

step "smoke test"
"$binary_path" --version >"$artifacts_dir/version.txt" 2>&1 ||
  fail "--version a échoué"
grep -Eq '^lunar [0-9]+\.[0-9]+\.[0-9]+' "$artifacts_dir/version.txt" ||
  fail "--version inattendu : $(cat "$artifacts_dir/version.txt")"
smoke_status=0
LUNAR_SMOKE_LOG_PATH="$artifacts_dir/startup-smoke-marqueurs.log" \
  timeout 90 "$binary_path" --lunar-smoke-test >"$artifacts_dir/startup-smoke.log" 2>&1 ||
  smoke_status=$?
((smoke_status == 0)) || fail "smoke test en échec (code $smoke_status, voir startup-smoke.log)"
for marker in LUNAR_STARTUP_VALIDATION=BEGIN LUNAR_SMOKE_FIRST_FRAME=OK LUNAR_STARTUP_VALIDATION=OK; do
  grep -Fxq "$marker" "$artifacts_dir/startup-smoke-marqueurs.log" ||
    fail "marqueur absent du smoke test : $marker"
done
pass "smoke test --lunar-smoke-test : première image, $(cat "$artifacts_dir/version.txt")"

# --- Ressources embarquées ---------------------------------------------------

step "ressources embarquées"
python3 -I - "$binary_path" "$ROOT_DIR/crates/lunar" >"$artifacts_dir/ressources-embarquees.txt" <<'PY' ||
import pathlib, re, sys
binary = pathlib.Path(sys.argv[1]).read_bytes()
crate = pathlib.Path(sys.argv[2])
listed = re.findall(r'"((?:icons|images)/[^"]+)"', (crate / "src/assets.rs").read_text())
listed.append("fonts/LiberationSans-Regular.ttf")
missing = [p for p in listed if (crate / "assets" / p).read_bytes() not in binary]
for p in listed:
    print(("ABSENT " if p in missing else "OK     ") + p)
print(f"{len(listed) - len(missing)}/{len(listed)} fichiers embarqués à l'octet")
sys.exit(1 if missing or len(listed) < 10 else 0)
PY
  fail "ressource absente du binaire (voir ressources-embarquees.txt)"
pass "ressources embarquées : $(tail -n1 "$artifacts_dir/ressources-embarquees.txt")"

# --- 2. Lancement et fenêtre ------------------------------------------------

step "lancement"
log "Lancement : $binary_path"
launch_app "$artifacts_dir/runtime.log"
network_sampler "$app_pid" "$artifacts_dir/reseau-releves.txt" &
sampler_pid=$!
if ((main_width < 1240 || main_height < 720)); then
  fail "fenêtre trop petite pour le scénario : ${main_width}x${main_height}"
fi
pass "binaire lancé, fenêtre LUNAR ${main_width}x${main_height} (PID $app_pid)"

# Icônes de la barre d'outils dessinées (assets embarqués présents) : la
# bande des boutons compte bien plus de couleurs
# que son fond uni et ses bordures.
toolbar_colors="$(wait_for_first_frame "$artifacts_dir/01-demarrage.png")"
((toolbar_colors >= 20)) ||
  fail "icônes de la barre d'outils non dessinées ($toolbar_colors couleurs)"
sleep 1
capture_window "01-demarrage"
pass "icônes rendues dans la barre d'outils ($toolbar_colors couleurs distinctes)"

assert_no_network "demarrage"

# --- 3. Panneau de configuration ---------------------------------------------

step "configuration"
mark="$(log_mark)"
send_ctrl comma
wait_for_log_since "$mark" "[Panneaux] config ouvert" 10
wait_for_log_since "$mark" "[Configuration] config=" 10
sleep 1
capture_window "02-configuration"
mark="$(log_mark)"
send_ctrl comma
wait_for_log_since "$mark" "[Panneaux] config fermé" 10
pass "panneau de configuration ouvert puis fermé (Ctrl+,)"

# --- 4. Panneau de filtre BPF -----------------------------------------------

step "filtre BPF"
mark="$(log_mark)"
send_ctrl f
wait_for_log_since "$mark" "[Panneaux] filtre ouvert" 10
sleep 1
capture_window "03-filtre-bpf"
mark="$(log_mark)"
send_ctrl f
wait_for_log_since "$mark" "[Panneaux] filtre fermé" 10
pass "panneau de filtre BPF ouvert puis fermé (Ctrl+F)"

# --- Capture live (option) --------------------------------------------------

if ((live_capture == 1)); then
  # shellcheck source-path=SCRIPTDIR source=live-capture-steps.sh
  source "$ROOT_DIR/script/e2e/live-capture-steps.sh"
else
  log "Capture live ignorée (ajouter --live-capture)."
fi

# --- 6. Import PCAP ---------------------------------------------------------

step "import PCAP"
# Panneau d'import (Ctrl+O), « Ajouter » une capture, sélecteur GTK,
# « Ouvrir » ; le panneau se ferme de lui-même à la fin de l'import.
import_file pcap "$pcap_fixture" "04-import-pcap"
wait_for_log_since "$import_mark" "10 paquets lus = 10 intégrés + 0 tronqués + 0 DLT non supportés + 0 malformés" 10
sleep 2
capture_window "05-pcap-importe"
pass "import PCAP réel : 10 paquets intégrés, graphe rechargé"

# Export CSV juste après l'import : identique à l'octet à parity_csv (même
# moteur, mêmes entrées), donc rien ne s'est perdu entre le moteur et l'UI.
step "export CSV (parité)"
parity_export="$artifacts_dir/matrice-pcap.csv"
parity_reference="$artifacts_dir/matrice-pcap-parity_csv.csv"
mark="$(log_mark)"
send_ctrl s
fill_file_dialog "$parity_export"
wait_for_log_since "$mark" "response: null" 20
wait_for_file "$parity_export"
"$parity_path" pcap "$parity_reference" "$pcap_fixture" >"$artifacts_dir/parity_csv.log" 2>&1 ||
  fail "parity_csv a échoué (voir parity_csv.log)"
cmp "$parity_export" "$parity_reference" ||
  fail "l'export CSV de l'interface diffère de parity_csv"
pass "export CSV identique à l'octet à parity_csv ($(($(wc -l <"$parity_export") - 2)) flux, $(sha256sum "$parity_export" | cut -c1-16)…)"

# --- 7. Gravité -------------------------------------------------------------

step "gravité"
mark="$(log_mark)"
click_main "$GRAPH_GRAVITY_X" "$GRAPH_CONTROLS_Y"
wait_for_log_since "$mark" "[Graphe] gravité" 10
first_state="$(log_line_since "$mark" "[Graphe] gravité" | sed -E 's/.*gravité ([a-zé]+).*/\1/')"
sleep 2
capture_window "06-gravite-$first_state"
mark="$(log_mark)"
click_main "$GRAPH_GRAVITY_X" "$GRAPH_CONTROLS_Y"
wait_for_log_since "$mark" "[Graphe] gravité" 10
second_state="$(log_line_since "$mark" "[Graphe] gravité" | sed -E 's/.*gravité ([a-zé]+).*/\1/')"
[[ "$first_state" != "$second_state" ]] ||
  fail "le bouton Gravité n'a pas basculé ($first_state puis $second_state)"
sleep 1
capture_window "06b-gravite-$second_state"
pass "gravité basculée deux fois ($first_state puis $second_state)"

# --- 9. Export PNG ----------------------------------------------------------

step "export PNG"
png_export="$artifacts_dir/graphe-export.png"
mark="$(log_mark)"
click_main "$GRAPH_EXPORT_X" "$GRAPH_CONTROLS_Y"
fill_file_dialog "$png_export"
wait_for_log_since "$mark" "PNG exporté dans $png_export" 30
wait_for_file "$png_export"
file "$png_export" | grep -Fq "PNG image data" || fail "l'export du graphe n'est pas un PNG valide"
read -r png_width png_height < <(identify -format '%w %h\n' "$png_export")
graph_width=$((main_width))
graph_height=$(($(side_bottom) - GRAPH_TOP))
# Export en 2 × la taille du canevas du graphe (EXPORT_SCALE) : toute la
# largeur, la hauteur de la zone moins la barre d'informations du graphe
# (zoom, nœuds, arêtes ; une quarantaine de pixels).
if ((png_width != graph_width * 2 ||
  png_height > graph_height * 2 || png_height < (graph_height - 60) * 2)); then
  fail "résolution PNG inattendue : ${png_width}x${png_height} (zone du graphe ${graph_width}x${graph_height})"
fi
png_colors="$(identify -format '%k' "$png_export")"
((png_colors > 2)) || fail "export PNG vide ($png_colors couleurs)"
pass "export PNG valide (${png_width}x${png_height}, $png_colors couleurs)"

# --- 10. Import du fichier de labels ----------------------------------------

step "import de labels"
mark="$(log_mark)"
send_ctrl_shift l
wait_for_log_since "$mark" "[Panneaux] import-labels ouvert" 10
sleep 1
capture_window "07-import-labels-panneau"
mark="$(log_mark)"
click_main "$(panel_center_x "$IMPORT_PANEL_W")" "$LABELS_ADD_FILE_Y"
fill_file_dialog "$labels_fixture"
wait_for_log_since "$mark" "import_label_file: $labels_fixture" 20
wait_for_log_since "$mark" "label(s) de nœud mis à jour dans le graphe" 20
sleep 1
capture_window "08-labels-importes"
pass "import du fichier de labels ($(log_line_since "$mark" "label(s) de nœud mis à jour" | sed -E 's/.*\] //'))"

# --- 11. Relevé WinAudit et arbitrage ----------------------------------------
# Le relevé nomme un poste de la
# capture (adresse et ligne de repli ajoutées), revendique la MAC et l'IP de
# « Passerelle » (désaccord : l'arbitrage s'ouvre) et porte un bouclage
# (écarté). Échap ferme l'arbitrage ; le label en place reste appliqué.

step "relevé WinAudit"
mark="$(log_mark)"
click_main "$(panel_center_x "$IMPORT_PANEL_W")" "$LABELS_WINAUDIT_Y"
fill_file_dialog "$winaudit_fixture"
wait_for_log_since "$mark" "[import_winaudit_surveys] 1 relevé(s) lu(s), 3 label(s) ajouté(s), 0 déjà présent(s), 1 conflit(s) à arbitrer" 20
wait_for_log_since "$mark" "[Panneaux] arbitrage ouvert" 10
sleep 1
capture_window "08b-winaudit-arbitrage"
mark="$(log_mark)"
send_key Escape
wait_for_log_since "$mark" "[Panneaux] arbitrage fermé" 10
sleep 1
pass "relevé WinAudit : 3 labels ajoutés, désaccord rendu à l'arbitrage (fermé par Échap)"

# --- 12. Gestion des labels -------------------------------------------------

step "gestion des labels"
mark="$(log_mark)"
send_ctrl k
wait_for_log_since "$mark" "[Panneaux] labels ouvert" 10
sleep 1
capture_window "09-gestion-labels"
mark="$(log_mark)"
send_ctrl k
wait_for_log_since "$mark" "[Panneaux] labels fermé" 10
pass "panneau de gestion des labels (Ctrl+K)"

# --- 13. Export CSV de la matrice -------------------------------------------

step "export CSV"
matrix_export="$artifacts_dir/matrice-export.csv"
mark="$(log_mark)"
send_ctrl s
fill_file_dialog "$matrix_export"
wait_for_log_since "$mark" "response: null" 20
wait_for_file "$matrix_export"
# Ligne 1 : commentaire « #LFMS version=2 … », ligne 2 : en-tête des colonnes.
head -n 1 "$matrix_export" | grep -q '^#LFMS version=' ||
  fail "commentaire #LFMS absent de l'export de la matrice"
sed -n 2p "$matrix_export" | grep -Fq "mac_source,mac_destination" ||
  fail "en-tête inattendu dans l'export de la matrice"
# Le poste nommé par le relevé WinAudit (28:cf:…, 192.168.88.52) est dans la
# capture : son label sort dans la colonne label_source de ses flux.
grep -Fq ",192.168.88.52,Privée,TST-PC-000001 POSTE-E2E-WINAUDIT," "$matrix_export" ||
  fail "le label tiré du relevé WinAudit n'apparaît pas dans l'export de la matrice"
pass "export CSV de la matrice (label WinAudit appliqué aux flux du poste)"

# --- 14. Export des labels --------------------------------------------------

step "export des labels"
labels_export="$artifacts_dir/labels-export.csv"
mark="$(log_mark)"
send_ctrl e
fill_file_dialog "$labels_export"
wait_for_log_since "$mark" "Labels exportés" 20
wait_for_file "$labels_export"
head -n 1 "$labels_export" | grep -Fxq "mac,ip,label" ||
  fail "en-tête inattendu dans l'export de labels"
for winaudit_line in \
  "28:cf:e9:18:b5:ed,192.168.88.52,TST-PC-000001 POSTE-E2E-WINAUDIT" \
  "44:15:24:20:a5:64,,TST-PC-000001 POSTE-E2E-WINAUDIT"; do
  grep -Fxq "$winaudit_line" "$labels_export" ||
    fail "label tiré du relevé WinAudit absent de l'export : $winaudit_line"
done
grep -Fxq "44:15:24:20:a5:64,192.168.1.254,Passerelle" "$labels_export" ||
  fail "le relevé WinAudit a écrasé « Passerelle » au lieu de l'envoyer à l'arbitrage"
pass "export CSV des labels (ajouts WinAudit, « Passerelle » intact)"

# --- 15. Export des journaux ------------------------------------------------

step "export des journaux"
logs_export="$artifacts_dir/logs-export.zip"
mark="$(log_mark)"
send_ctrl l
fill_file_dialog "$logs_export"
wait_for_log_since "$mark" "Sauvegarde terminée" 30
wait_for_file "$logs_export"
file "$logs_export" | grep -Fq "Zip archive data" ||
  fail "l'export des journaux n'est pas une archive ZIP"
unzip -Z1 "$logs_export" >"$artifacts_dir/logs-export-contenu.txt"
grep -q '\.log$' "$artifacts_dir/logs-export-contenu.txt" ||
  fail "l'archive de journaux ne contient aucun fichier .log"
pass "export des journaux en archive ZIP ($(wc -l <"$artifacts_dir/logs-export-contenu.txt") fichiers)"

assert_no_network "exports"

# --- 16. Réinitialisation puis import d'une matrice -------------------------

# La matrice est déposée sur le panneau d'import (glisser-déposer XDND, le
# pilote jouant la source) au lieu de passer par « Ajouter » : le sélecteur
# GTK est déjà couvert par les autres imports et exports.
step "réinitialisation et import de matrice par glisser-déposer"
reset_capture "09b-confirmation-reinitialisation"
sleep 1
dialogs_before="$dialog_count"
import_by_drop=1
import_file matrix "$matrix_fixture" "09c-import-matrice"
import_by_drop=""
((dialog_count == dialogs_before)) || fail "un sélecteur s'est ouvert pendant le glisser-déposer"
wait_for_log_since "$import_mark" "30 ligne(s) importée(s) -> 30 flux fusionné(s), 12 nœuds, 11 arêtes" 10
sleep 2
capture_window "10-matrice-importee"
pass "réinitialisation confirmée puis matrice CSV déposée sur le panneau d'import et importée (12 nœuds, 11 arêtes)"

# --- 17. Vue d'investigation ------------------------------------------------

step "vue d'investigation"
investigation_label="E2E-INVESTIGATION"
mark="$(log_mark)"
send_ctrl i
wait_for_log_since "$mark" "[Investigation] snapshot chargé" 20
sleep 2
capture_window "11-investigation-resume"
click_main "$INVESTIGATION_ENDPOINTS_TAB_X" "$INVESTIGATION_TAB_Y"
sleep 1
capture_window "12-investigation-endpoints"
# Clic sur la première ligne (colonne Nom, à gauche) : la ligne est
# sélectionnée et le tableau prend le focus ; Entrée ouvre l'édition de son
# label (TableEvent::Activated), Ctrl+A puis la saisie le remplacent, Entrée
# l'enregistre.
click_main "$INVESTIGATION_FIRST_ROW_X" "$INVESTIGATION_FIRST_ROW_Y"
sleep 0.5
send_key Return
sleep 0.5
send_ctrl a
type_text "$investigation_label"
sleep 0.3
send_key Return
sleep 2
capture_window "13-investigation-label-saisi"
pass "vue d'investigation : instantané chargé, label saisi au clavier"

# --- 18. Export de synthèse -------------------------------------------------

step "export de synthèse"
synthesis_html="$artifacts_dir/synthese-investigation.html"
mark="$(log_mark)"
send_ctrl s
fill_file_dialog "$synthesis_html"
wait_for_log_since "$mark" "[Investigation] synthèse exportée : $synthesis_html" 30
wait_for_file "$synthesis_html"
synthesis_line="$(log_line_since "$mark" "[Investigation] synthèse exportée")"
synthesis_csv_name="$(sed -E 's/.*, matrice (.*), SHA-256 .*/\1/' <<<"$synthesis_line")"
logged_sha="$(sed -E 's/.*SHA-256 ([0-9a-f]{64}).*/\1/' <<<"$synthesis_line")"
synthesis_csv="$artifacts_dir/$synthesis_csv_name"
wait_for_file "$synthesis_csv"
synthesis_sha="$(sha256sum "$synthesis_csv" | cut -d' ' -f1)"
[[ "$synthesis_sha" == "$logged_sha" ]] ||
  fail "empreinte journalisée ($logged_sha) ≠ empreinte du CSV ($synthesis_sha)"
grep -Fq "<span class=\"hash\">$synthesis_sha</span>" "$synthesis_html" ||
  fail "la synthèse HTML ne porte pas l'empreinte du CSV écrit à côté"
if grep -Eiq '<script|<[^>]*[[:space:]]on[a-z]+[[:space:]]*=' "$synthesis_html"; then
  fail "la synthèse HTML contient du script"
fi
grep -Fq "$investigation_label" "$synthesis_html" ||
  fail "le label posé dans la vue est absent de la synthèse"
grep -Fq '<h1>Synthèse du relevé LUNAR</h1>' "$synthesis_html" ||
  fail "la synthèse ne porte pas le nom de l'application (LUNAR)"
grep -Eq '<meta name="generator" content="LUNAR [0-9]+\.[0-9]+\.[0-9]+">' "$synthesis_html" ||
  fail "générateur LUNAR absent de la synthèse"
grep -Fq '<b>inconnue</b><span>Durée du relevé</span>' "$synthesis_html" ||
  fail "la durée d'un relevé importé sans période n'est pas « inconnue »"
sleep 1
capture_window "13b-investigation-synthese-exportee"
pass "export de synthèse HTML LUNAR : sans script, SHA-256 du CSV joint, label présent"

# --- 19. Retour à l'analyse -------------------------------------------------

step "retour à l'analyse"
mark="$(log_mark)"
send_ctrl i
wait_for_log_since "$mark" "[Vue] analyse" 15
wait_for_log_since "$mark" "[Barre d'état] Restauration au montage" 15
wait_for_log_since "$mark" "[Graphe] Restauration au montage : 12 nœuds" 15
if log_line_since "$mark" "[Barre d'état] Restauration au montage" | grep -Fq ": 0 reçus, 0 flux,"; then
  restoration="$(log_line_since "$mark" "[Barre d'état] Restauration au montage")"
  grep -Fq "import " <<<"$restoration" ||
    fail "la barre de statut est repartie de zéro au retour dans l'analyse"
fi
sleep 2
capture_window "14-retour-analyse"
pass "retour à l'analyse (Ctrl+I), graphe intact (12 nœuds) : $(log_line_since "$mark" "[Barre d'état] Restauration" | sed -E 's/.*\] //')"

# --- 20. Label de l'investigation dans l'export -----------------------------

step "export des labels après investigation"
investigation_labels="$artifacts_dir/labels-apres-investigation.csv"
mark="$(log_mark)"
send_ctrl e
fill_file_dialog "$investigation_labels"
wait_for_log_since "$mark" "Labels exportés" 20
wait_for_file "$investigation_labels"
grep -Fq "$investigation_label" "$investigation_labels" ||
  fail "le label posé depuis la vue d'investigation est absent de l'export"
capture_window "15-analyse-apres-export"
pass "label de la vue d'investigation retrouvé dans l'export des labels"

# --- 21. Journaux sains, réseau, fermeture ----------------------------------

step "journaux et réseau"
assert_no_network "fin"
if grep -Eiq 'panicked|fatal runtime error' "$runtime_log" "$artifacts_dir/startup-smoke.log"; then
  fail "panic détectée dans les journaux"
fi
if [[ -s "$artifacts_dir/reseau-releves.txt" ]]; then
  while read -r _ proto local_addr remote_addr; do
    if ! is_loopback_hex "$local_addr" || ! is_loopback_hex "$remote_addr"; then
      fail "socket hors boucle locale relevée pendant la session : $proto $local_addr -> $remote_addr"
    fi
  done <"$artifacts_dir/reseau-releves.txt"
fi
samples="$(grep -c . "$artifacts_dir/reseau-releves.txt" 2>/dev/null || true)"
kinds="$(awk '{print $2}' "$artifacts_dir/reseau-releves.txt" 2>/dev/null | sort -u | tr '\n' ' ')"
pass "aucune panic ; aucune socket réseau hors boucle locale (${samples:-0} relevé(s) de socket : ${kinds:-aucune}, toutes locales)"

step "fermeture"
app_status=0
quit_app "16-confirmation-fermeture" || app_status=$?
((app_status == 0)) || fail "code de sortie $app_status à la fermeture"
grep -Fq "LUNAR fermé" "$runtime_log" || fail "« LUNAR fermé » absent du journal"
pass "fermeture confirmée (Ctrl+Q) : code 0, « LUNAR fermé »"

log "Scénario terminé : $pass_count contrôles réussis."
