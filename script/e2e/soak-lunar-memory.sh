#!/usr/bin/env bash
# Mémoire de LUNAR sur la durée.
#
# Le binaire release tourne sur un écran Xvfb dédié, dans le même
# isolement que le parcours E2E (espace de noms utilisateur + réseau, bus
# D-Bus minimal, XDG_* et HOME dans les artefacts). Chaque cycle importe une
# matrice (ou une capture) par le panneau d'import, laisse la gravité
# (ForceAtlas2) tourner un moment, puis réinitialise le relevé. VmRSS,
# VmHWM et le nombre de threads du processus sont relevés après chaque phase
# dans memoire.csv, avec, après la réinitialisation, le tas vivant que LUNAR
# journalise (`[Mémoire]`, mallinfo2 : octets alloués non libérés).
#
# Le fichier est déposé sur le panneau (glisser-déposer XDND, défaut) :
# chaque sélecteur GTK ouvert par rfd garde environ 1,6 Mo (mesuré, voir
# README), ce qui masquerait une fuite propre à LUNAR. `--via dialog` passe
# par le sélecteur pour mesurer ce coût.
#
# Verdict, sur les relevés pris après la réinitialisation (état « vide »
# comparable d'un cycle à l'autre), passé les cycles de chauffe :
#   - croissance : dernier relevé − premier relevé ≤ --tolerance-mb ;
#   - pente : régression linéaire ≤ --max-slope-mb par cycle ;
#   - tas vivant : pente ≤ --max-heap-slope-mb par cycle (une fuite y
#     apparaît sans le bruit de la fragmentation) ;
#   - threads : au plus --max-thread-growth de plus qu'au premier relevé.
# Les cycles de chauffe sont écartés : l'allocateur (arènes glibc), les
# caches de glyphes et d'atlas de gpui et le pilote Vulkan logiciel
# (lavapipe) montent en régime pendant les premiers cycles puis plafonnent.
#
# Le script n'arrête que les processus qu'il a lancés et ne touche ni
# l'écran ni les données de l'utilisateur. Voir script/e2e/README.md.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_PATH="$ROOT_DIR/script/e2e/$(basename "${BASH_SOURCE[0]}")"
fixtures_dir="$ROOT_DIR/fixtures"

binary_path="$ROOT_DIR/target/release/lunar"
artifacts_dir=""
display_name=":122"
cycles=20
warmup=5
gravity_seconds=15
input_kind="matrix"
import_via="drop"
fixture=""
tolerance_mb=48
max_slope_mb=1.5
max_heap_slope_mb=0.5
max_thread_growth=2

usage() {
  cat <<'EOF'
Usage: soak-lunar-memory.sh [options]

Options:
  --binary PATH            Binaire lunar (défaut : target/release/lunar).
  --artifacts DIR          Dossier des relevés ; doit être absent ou vide
                           (défaut : dossier temporaire).
  --display :N             Écran Xvfb dédié, refusé s'il est occupé (défaut : :122).
  --cycles N               Cycles import → gravité → réinitialisation (défaut : 20).
  --warmup N               Cycles de chauffe écartés du verdict (défaut : 5).
  --gravity-seconds S      Durée de la gravité à chaque cycle (défaut : 15).
  --input matrix|pcap      Import d'une matrice CSV (défaut) ou d'une capture.
  --via drop|dialog        Ajout du fichier par glisser-déposer (défaut) ou par
                           le sélecteur GTK (« Ajouter »).
  --fixture PATH           Fichier importé (défaut : 20260703_NP_Matrice_1000.csv
                           ou pcap_tshark_corpus/linux_sll.pcap).
  --tolerance-mb M         Croissance tolérée après la chauffe (défaut : 48).
  --max-slope-mb M         Pente tolérée en Mo par cycle (défaut : 1.5).
  --max-heap-slope-mb M    Pente tolérée du tas vivant en Mo par cycle (défaut : 0.5).
  --max-thread-growth N    Threads supplémentaires tolérés (défaut : 2).
  -h, --help               Affiche cette aide.
EOF
}

while (($# > 0)); do
  case "$1" in
    --binary) binary_path="${2:?--binary attend un chemin}"; shift 2 ;;
    --artifacts) artifacts_dir="${2:?--artifacts attend un chemin}"; shift 2 ;;
    --display) display_name="${2:?--display attend :N}"; shift 2 ;;
    --cycles) cycles="${2:?--cycles attend un entier}"; shift 2 ;;
    --warmup) warmup="${2:?--warmup attend un entier}"; shift 2 ;;
    --gravity-seconds) gravity_seconds="${2:?--gravity-seconds attend un entier}"; shift 2 ;;
    --input) input_kind="${2:?--input attend matrix ou pcap}"; shift 2 ;;
    --via) import_via="${2:?--via attend drop ou dialog}"; shift 2 ;;
    --fixture) fixture="${2:?--fixture attend un chemin}"; shift 2 ;;
    --tolerance-mb) tolerance_mb="${2:?--tolerance-mb attend un nombre}"; shift 2 ;;
    --max-slope-mb) max_slope_mb="${2:?--max-slope-mb attend un nombre}"; shift 2 ;;
    --max-heap-slope-mb) max_heap_slope_mb="${2:?--max-heap-slope-mb attend un nombre}"; shift 2 ;;
    --max-thread-growth) max_thread_growth="${2:?--max-thread-growth attend un entier}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Option inconnue : $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$display_name" =~ ^:[1-9][0-9]*$ ]] || { echo "--display attend :N" >&2; exit 2; }
for value in "$cycles" "$gravity_seconds"; do
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || { echo "entier strictement positif attendu : $value" >&2; exit 2; }
done
[[ "$warmup" =~ ^[0-9]+$ && "$max_thread_growth" =~ ^[0-9]+$ ]] ||
  { echo "--warmup et --max-thread-growth attendent un entier" >&2; exit 2; }
((cycles - warmup >= 3)) || { echo "au moins 3 cycles après la chauffe" >&2; exit 2; }
for value in "$tolerance_mb" "$max_slope_mb" "$max_heap_slope_mb"; do
  [[ "$value" =~ ^[0-9]+([.][0-9]+)?$ ]] ||
    { echo "--tolerance-mb, --max-slope-mb et --max-heap-slope-mb attendent un nombre" >&2; exit 2; }
done
[[ "$import_via" == drop || "$import_via" == dialog ]] ||
  { echo "--via attend drop ou dialog" >&2; exit 2; }
case "$input_kind" in
  matrix) fixture="${fixture:-$fixtures_dir/20260703_NP_Matrice_1000.csv}" ;;
  pcap) fixture="${fixture:-$fixtures_dir/pcaps/import/pcap_tshark_corpus/linux_sll.pcap}" ;;
  *) echo "--input attend matrix ou pcap" >&2; exit 2 ;;
esac

log() {
  printf '[LUNAR SOAK] %s\n' "$*"
}

fail() {
  log "ÉCHEC — $*" >&2
  exit 1
}

# shellcheck source-path=SCRIPTDIR source=lunar-x11-lib.sh
source "$ROOT_DIR/script/e2e/lunar-x11-lib.sh"

if [[ "${LUNAR_E2E_STAGE:-host}" == "host" ]]; then
  display_is_free || { echo "l'écran $display_name est occupé : choisir --display" >&2; exit 1; }
  prepare_artifacts_dir lunar-soak
  binary_path="$(absolute_path "$binary_path")"
  fixture="$(absolute_path "$fixture")"
fi
enter_isolation "$SCRIPT_PATH" --binary "$binary_path" --artifacts "$artifacts_dir" \
  --display "$display_name" --cycles "$cycles" --warmup "$warmup" \
  --gravity-seconds "$gravity_seconds" --input "$input_kind" --via "$import_via" \
  --fixture "$fixture" \
  --tolerance-mb "$tolerance_mb" --max-slope-mb "$max_slope_mb" \
  --max-heap-slope-mb "$max_heap_slope_mb" --max-thread-growth "$max_thread_growth"

# --- Session ----------------------------------------------------------------

samples="$artifacts_dir/memoire.csv"
summary_file="$artifacts_dir/summary.txt"
verdict_file="$artifacts_dir/verdict.txt"
current_cycle=0

cleanup() {
  local status="$?"
  set +e
  if ((status != 0)) && [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    capture_window "echec" >/dev/null 2>&1
  fi
  stop_pid "$app_pid"
  stop_pid "$xvfb_pid"
  {
    printf 'Binaire : %s\n' "$binary_path"
    printf 'Écran : %s (Xvfb dédié), réseau : espace de noms isolé (lo seule)\n' "$display_name"
    printf 'Import : %s (%s, ajout par %s)\n' "$fixture" "$input_kind" \
      "$([[ "$import_via" == drop ]] && echo glisser-déposer || echo "sélecteur GTK")"
    printf 'Cycles : %d dont %d de chauffe ; gravité %d s par cycle\n' \
      "$cycles" "$warmup" "$gravity_seconds"
    printf 'Tolérances : croissance ≤ %s Mo, pente ≤ %s Mo/cycle, tas vivant ≤ %s Mo/cycle, threads ≤ +%d\n\n' \
      "$tolerance_mb" "$max_slope_mb" "$max_heap_slope_mb" "$max_thread_growth"
    [[ -s "$verdict_file" ]] && cat "$verdict_file"
    if ((status == 0)); then
      printf 'LUNAR SOAK MÉMOIRE: PASS\n'
    else
      printf 'Arrêt au cycle %d (code %d)\n' "$current_cycle" "$status"
      printf 'LUNAR SOAK MÉMOIRE: FAIL\n'
    fi
  } >"$summary_file"
  cat "$summary_file"
  log "Relevés : $samples"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

for command_name in cc pkg-config Xvfb xdpyinfo import identify convert python3; do
  command -v "$command_name" >/dev/null 2>&1 || fail "commande requise absente : $command_name"
done
[[ -x "$binary_path" ]] || fail "binaire introuvable ou non exécutable : $binary_path"
[[ -f "$fixture" ]] || fail "fichier à importer introuvable : $fixture"

compile_input_driver "$ROOT_DIR"
start_xvfb
export_isolated_env
launch_app "$artifacts_dir/runtime.log"
colors="$(wait_for_first_frame "$artifacts_dir/00-demarrage.png")"
((colors >= 20)) || fail "première image non dessinée"

# Relevé /proc du processus : VmRSS, VmHWM (Ko), Threads ; HEAP : tas
# vivant en Ko (vide hors réinitialisation).
sample() {
  local cycle="$1" phase="$2" heap="${3:-}" rss hwm threads
  rss="$(awk '/^VmRSS:/ {print $2}' "/proc/$app_pid/status")"
  hwm="$(awk '/^VmHWM:/ {print $2}' "/proc/$app_pid/status")"
  threads="$(awk '/^Threads:/ {print $2}' "/proc/$app_pid/status")"
  printf '%d,%s,%d,%d,%d,%d,%s\n' "$cycle" "$phase" "$SECONDS" "$rss" "$hwm" "$threads" \
    "$heap" >>"$samples"
  log "cycle $cycle $phase : VmRSS $((rss / 1024)) Mo, threads $threads${heap:+, tas vivant $((heap / 1024)) Mo}"
}

[[ "$import_via" == dialog ]] || import_by_drop=1
echo "cycle,phase,secondes,vmrss_ko,vmhwm_ko,threads,tas_vivant_ko" >"$samples"
sleep 2
sample 0 depart

for ((current_cycle = 1; current_cycle <= cycles; current_cycle++)); do
  shot=""
  if ((current_cycle == 1 || current_cycle == cycles)); then
    shot="cycle-$(printf %02d "$current_cycle")"
  fi
  import_file "$input_kind" "$fixture" "$shot"
  sleep 1
  sample "$current_cycle" import
  set_gravity activée >/dev/null
  sleep "$gravity_seconds"
  [[ -z "$shot" ]] || capture_window "$shot-gravite"
  sample "$current_cycle" gravite
  mark="$(log_mark)"
  reset_capture ""
  # Mémoire rendue au système par LUNAR après la réinitialisation, tas vivant.
  wait_for_log_since "$mark" "[Mémoire] pages libres rendues" 10
  heap="$(log_line_since "$mark" "[Mémoire] pages libres rendues" |
    sed -E 's/.*allocations vivantes ([0-9]+) Ko.*/\1/')"
  sleep 2
  sample "$current_cycle" reinitialisation "$heap"
done
current_cycle=$cycles

if grep -Eiq 'panicked|fatal runtime error' "$runtime_log"; then
  fail "panic détectée dans le journal"
fi
app_status=0
quit_app "" || app_status=$?
((app_status == 0)) || fail "code de sortie $app_status à la fermeture"

# Verdict et courbe (relevés après réinitialisation).
python3 -I - "$samples" "$warmup" "$tolerance_mb" "$max_slope_mb" "$max_thread_growth" \
  "$max_heap_slope_mb" >"$verdict_file" <<'PY' || fail "mémoire hors tolérance (voir verdict ci-dessous)"
import csv, sys
path, warmup = sys.argv[1], int(sys.argv[2])
tolerance, max_slope, max_threads = float(sys.argv[3]), float(sys.argv[4]), int(sys.argv[5])
max_heap_slope = float(sys.argv[6])
rows = list(csv.DictReader(open(path, newline="")))
resets = [r for r in rows if r["phase"] == "reinitialisation"]
peaks = {r["cycle"]: int(r["vmrss_ko"]) for r in rows if r["phase"] == "gravite"}
top = max(int(r["vmrss_ko"]) for r in resets)
print("cycle  après réinit.  tas vivant  pic (gravité)  threads")
for r in resets:
    mb = int(r["vmrss_ko"]) / 1024
    bar = "#" * max(1, round(40 * int(r["vmrss_ko"]) / top))
    mark = " (chauffe)" if int(r["cycle"]) <= warmup else ""
    heap = f"{int(r['tas_vivant_ko']) / 1024:7.1f} Mo" if r["tas_vivant_ko"] else "      ? Mo"
    print(f"{int(r['cycle']):>5}  {mb:9.1f} Mo  {heap}  {peaks.get(r['cycle'], 0) / 1024:9.1f} Mo"
          f"  {int(r['threads']):>7}  {bar}{mark}")
kept = [r for r in resets if int(r["cycle"]) > warmup]

def fit(xs, ys):
    mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)

xs = [int(r["cycle"]) for r in kept]
ys = [int(r["vmrss_ko"]) / 1024 for r in kept]
slope = fit(xs, ys)
growth = ys[-1] - ys[0]
heap_rows = [r for r in kept if r["tas_vivant_ko"]]
heap_ok = len(heap_rows) == len(kept)
if heap_ok:
    heap_ys = [int(r["tas_vivant_ko"]) / 1024 for r in heap_rows]
    heap_slope = fit([int(r["cycle"]) for r in heap_rows], heap_ys)
threads = [int(r["threads"]) for r in kept]
thread_growth = max(threads) - threads[0]
first_hwm = int(rows[0]["vmhwm_ko"]) / 1024
last_hwm = int(rows[-1]["vmhwm_ko"]) / 1024
print()
print(f"Après chauffe (cycles {xs[0]} à {xs[-1]}) : {ys[0]:.1f} Mo -> {ys[-1]:.1f} Mo, "
      f"croissance {growth:+.1f} Mo (tolérance {tolerance:g}), "
      f"pente {slope:+.2f} Mo/cycle (tolérance {max_slope:g}), "
      f"min {min(ys):.1f} / max {max(ys):.1f} Mo")
if heap_ok:
    print(f"Tas vivant après chauffe : {heap_ys[0]:.1f} Mo -> {heap_ys[-1]:.1f} Mo, "
          f"pente {heap_slope:+.2f} Mo/cycle (tolérance {max_heap_slope:g})")
else:
    print("Tas vivant : relevé manquant (journal [Mémoire] absent)")
print(f"Threads après chauffe : {threads[0]} -> {threads[-1]} (max {max(threads)}, "
      f"tolérance +{max_threads})")
print(f"VmHWM (pic du processus) : {first_hwm:.1f} Mo au départ, {last_hwm:.1f} Mo à la fin")
ok = (growth <= tolerance and slope <= max_slope and thread_growth <= max_threads
      and heap_ok and heap_slope <= max_heap_slope)
print("Verdict mémoire : " + ("dans les tolérances" if ok else "HORS TOLÉRANCE"))
sys.exit(0 if ok else 1)
PY
log "Mesure terminée : $cycles cycles."
