# shellcheck shell=bash disable=SC2154
# Étapes de capture live du parcours E2E de LUNAR (--live-capture).
#
# Fichier lu par run-lunar-x11-e2e.sh (`source`), dans la session de
# test : ses fonctions et variables (main_id, main_width, artifacts_dir,
# mark, fill_file_dialog, la géométrie…) sont celles du harnais. Il ne se
# lance pas seul.
#
# L'espace de noms réseau du harnais ne contient que `lo` et y donne
# CAP_NET_RAW sans sudo : LUNAR capture réellement sur `lo`, filtre BPF
# `icmp` posé, pendant qu'un ping local tourne. Un peu d'UDP sans écoute est
# envoyé en même temps : les datagrammes sont écartés par le filtre, seuls
# restent les ICMP « port inaccessible » que le noyau renvoie.

# Indice d'une interface dans la liste journalisée par `get_interfaces`
# (« Interfaces réseau: [ … name: "lo", … ] »), qui est aussi l'ordre du
# sélecteur du panneau de configuration.
live_interface_index() {
  awk -v wanted="$1" '
    /Interfaces réseau: \[/ { listing = 1; index_ = 0; found = -1; next }
    listing && /^\]/ { listing = 0; if (found >= 0) last = found }
    listing && /^        name: "/ {
      name = $0; sub(/^ *name: "/, "", name); sub(/",$/, "", name)
      if (name == wanted && found < 0) found = index_
      index_++
    }
    END { print (last == "" ? -1 : last) }' "$runtime_log"
}

step "capture live : interface lo"
mark="$(log_mark)"
send_ctrl comma
wait_for_log_since "$mark" "[Panneaux] config ouvert" 10
wait_for_log_since "$mark" "[Configuration] config=" 10
wait_for_log_since "$mark" "Interfaces réseau: [" 10
sleep 1
current_device="$(log_line_since "$mark" "[Configuration] config=" |
  sed -E 's/.*"device_name":"([^"]*)".*/\1/')"
lo_index="$(live_interface_index lo)"
current_index="$(live_interface_index "$current_device")"
((lo_index >= 0)) || fail "lo absente de la liste des interfaces"
((current_index >= 0)) || current_index=0
log "Interface configurée : $current_device (rang $current_index), lo au rang $lo_index"
# Le sélecteur s'ouvre sur l'entrée choisie : flèches jusqu'à lo, Entrée.
click_main "$(panel_x "$CONFIG_PANEL_W" "$CONFIG_SELECT_DX")" "$CONFIG_SELECT_Y"
sleep 1
capture_window "03b-live-interfaces"
moves=$((lo_index - current_index))
while ((moves > 0)); do send_key Down; sleep 0.2; moves=$((moves - 1)); done
while ((moves < 0)); do send_key Up; sleep 0.2; moves=$((moves + 1)); done
send_key Return
sleep 1
# « Sauvegarder » est en bas du panneau : défilement jusqu'au bout.
wheel_main "$(panel_center_x "$CONFIG_PANEL_W")" $(($(side_bottom) / 2 + GRAPH_TOP / 2)) down 20
sleep 1
capture_window "03c-live-configuration-lo"
mark="$(log_mark)"
click_until_log "$(panel_x "$CONFIG_PANEL_W" "$CONFIG_SAVE_DX")" \
  $(($(side_bottom) - CONFIG_SAVE_DY)) '[get_config_capture] app.config "lo"' "$mark"
wait_for_log_since "$mark" "[Panneaux] config fermé" 10
pass "capture live : interface lo choisie et enregistrée (panneau de configuration)"

step "capture live : filtre icmp"
mark="$(log_mark)"
send_ctrl f
wait_for_log_since "$mark" "[Panneaux] filtre ouvert" 10
sleep 1
click_main "$(panel_x "$FILTER_PANEL_W" "$FILTER_ICMP_DX")" "$FILTER_ICMP_Y"
sleep 0.5
wheel_main "$(panel_center_x "$FILTER_PANEL_W")" $(($(side_bottom) / 2 + GRAPH_TOP / 2)) down 30
sleep 1
capture_window "03d-live-filtre-icmp"
mark="$(log_mark)"
click_until_log "$(panel_x "$FILTER_PANEL_W" "$FILTER_APPLY_DX")" \
  $(($(side_bottom) - FILTER_APPLY_DY)) "[set_filter] filter: icmp" "$mark"
wait_for_log_since "$mark" "[Panneaux] filtre fermé" 10
log_line_since "$mark" "[set_filter] filter:" | grep -Eq 'filter: icmp$' ||
  fail "filtre appliqué différent de « icmp » : $(log_line_since "$mark" "[set_filter] filter:")"
pass "capture live : filtre BPF « icmp » appliqué (case ICMP, Appliquer)"

step "capture live : démarrage et trafic"
sleep 1
capture_window "03e-live-avant"
packet_rows_crop="${main_width}x$((PACKET_LOG_H - 40))+0+$(($(side_bottom) + 34))"
rows_before="$(convert "$artifacts_dir/03e-live-avant.png" -crop "$packet_rows_crop" +repage -format '%k' info:)"
mark="$(log_mark)"
send_ctrl p
wait_for_log_since "$mark" "Capture démarrée : true" 20
wait_for_log_since "$mark" "Interface trouvée : lo" 10
sleep 1
ping_count=$((live_seconds * 5))
timeout $((live_seconds + 10)) ping -n -c "$ping_count" -i 0.2 127.0.0.1 \
  >"$artifacts_dir/live-ping.txt" 2>&1 || fail "ping local en échec (voir live-ping.txt)"
for _ in 1 2 3; do
  printf 'lunar-e2e\n' >/dev/udp/127.0.0.1/9 || true
  sleep 0.1
done
sleep 3
capture_window "03f-live-capture-en-cours"
rows_during="$(convert "$artifacts_dir/03f-live-capture-en-cours.png" -crop "$packet_rows_crop" +repage -format '%k' info:)"
((rows_during > rows_before + 5)) ||
  fail "journal de trames vide pendant la capture ($rows_before puis $rows_during couleurs)"
mark="$(log_mark)"
send_ctrl_shift p
wait_for_log_since "$mark" "Capture arrêtée : false" 20
wait_for_log_since "$mark" "Thread de capture terminé." 10
sleep 1
capture_window "03g-live-arretee"
pass "capture live démarrée sur lo puis arrêtée (Ctrl+P, Ctrl+Shift+P) ; $(tail -n1 "$artifacts_dir/live-ping.txt" | cut -c1-60)"

# Compteurs : la barre de statut écrit « reçus, flux » en revenant dans la
# vue d'analyse (aller-retour par la vue d'investigation).
step "capture live : compteurs"
mark="$(log_mark)"
send_ctrl i
wait_for_log_since "$mark" "[Vue] investigation" 15
sleep 2
mark="$(log_mark)"
send_ctrl i
wait_for_log_since "$mark" "[Barre d'état] Restauration au montage" 15
wait_for_log_since "$mark" "[Graphe] Restauration au montage" 15
live_counters="$(log_line_since "$mark" "[Barre d'état] Restauration au montage")"
received="$(sed -E 's/.*: ([0-9]+) reçus, ([0-9]+) flux.*/\1/' <<<"$live_counters")"
processed="$(sed -E 's/.*: ([0-9]+) reçus, ([0-9]+) flux.*/\2/' <<<"$live_counters")"
# reçus = trames comptées par libpcap (au moins requête + réponse par
# ping), flux = flux distincts de la matrice (un seul ici : ICMP 127.0.0.1).
((received >= 2 * ping_count && processed >= 1)) ||
  fail "compteurs trop bas pour $ping_count pings : $live_counters"
pass "capture live : compteurs $received reçus (≥ 2 × $ping_count pings), $processed flux"

step "capture live : export CSV"
live_export="$artifacts_dir/capture-live.csv"
mark="$(log_mark)"
send_ctrl s
fill_file_dialog "$live_export"
wait_for_log_since "$mark" "response: null" 20
wait_for_file "$live_export"
head -n 1 "$live_export" | grep -q '^#LFMS version=2 dlt=ETHERNET interface=lo ' ||
  fail "l'export de la capture live ne porte pas interface=lo"
python3 -I - "$live_export" "$ping_count" >"$artifacts_dir/capture-live-controle.txt" <<'PY' ||
import csv, sys
path, pings = sys.argv[1], int(sys.argv[2])
with open(path, newline="") as f:
    rows = list(csv.DictReader(line for line in f if not line.startswith("#")))
protocols = sorted({r["protocol_transport"] for r in rows})
icmp = sum(int(r["count"]) for r in rows if r["protocol_transport"] == "ICMP")
print(f"{len(rows)} flux, protocoles {protocols}, {icmp} paquets ICMP")
ok = rows and protocols == ["ICMP"] and icmp >= 2 * pings and all(
    r["ip_source"] == r["ip_destination"] == "127.0.0.1" for r in rows)
sys.exit(0 if ok else 1)
PY
  fail "export de la capture live inattendu : $(cat "$artifacts_dir/capture-live-controle.txt")"
pass "capture live exportée : $(cat "$artifacts_dir/capture-live-controle.txt") (UDP écarté par le filtre)"

# Relevé remis à zéro avant l'import PCAP, dont l'export doit rester
# identique à parity_csv.
step "capture live : réinitialisation"
reset_capture ""
sleep 1
pass "relevé de la capture live réinitialisé (Ctrl+Shift+R)"
