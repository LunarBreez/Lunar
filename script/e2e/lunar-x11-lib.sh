# shellcheck shell=bash disable=SC2034,SC2154
# Fonctions communes aux scripts E2E de LUNAR sous X11 :
# run-lunar-x11-e2e.sh (parcours) et soak-lunar-memory.sh
# (mémoire sur la durée). Fichier lu par `source`, il ne se lance pas seul.
#
# Le script appelant définit avant de le lire `log MESSAGE` et
# `fail MESSAGE` (qui sort en erreur), et les variables `artifacts_dir`
# (dossier des preuves, absolu), `display_name` (:N) et, en session,
# `binary_path`. Les fonctions tiennent à jour `input_binary`, `xvfb_pid`,
# `app_pid`, `main_id`, `main_width`, `main_height`, `runtime_log` et
# `dialog_count`.
#
# Isolation : tout se déroule dans un espace de noms utilisateur + réseau
# (`unshare -rn`, seule `lo` existe, CAP_NET_RAW sans sudo), sur un écran
# Xvfb propre, avec un bus D-Bus de session minimal et des XDG_* et HOME
# dans le dossier d'artefacts. Seuls les PID lancés ici sont arrêtés.

input_binary=""
xvfb_pid=""
app_pid=""
main_id=""
main_width=""
main_height=""
runtime_log=""
dialog_count=0
import_mark=0

# --- Amorçage : hôte → espace de noms → session -----------------------------

# Écran :N libre (ni socket ni verrou) : jamais celui de l'utilisateur.
display_is_free() {
  local number="${display_name#:}"
  [[ ! -e "/tmp/.X11-unix/X$number" && ! -e "/tmp/.X${number}-lock" ]]
}

# Ré-exécute SCRIPT avec ARGS… à l'étape suivante, ou rend la main en
# session. Hôte : entrée dans un espace de noms utilisateur + réseau, sans
# rien hériter de l'écran ni de la session de l'utilisateur. Espace de
# noms : `lo` levée, puis un bus de session minimal sans répertoire de
# services (rien n'y est activé à la demande : ni portail, ni gvfs, ni
# at-spi ; les sélecteurs GTK n'ont besoin que du bus).
enter_isolation() {
  local script="$1"
  shift
  case "${LUNAR_E2E_STAGE:-host}" in
    host)
      local command_name
      for command_name in unshare ip dbus-run-session; do
        command -v "$command_name" >/dev/null 2>&1 || {
          echo "commande requise absente : $command_name" >&2
          exit 1
        }
      done
      exec env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS \
        -u XAUTHORITY -u SESSION_MANAGER -u XDG_SESSION_TYPE \
        LUNAR_E2E_STAGE=netns \
        unshare --user --map-root-user --net -- "$script" "$@"
      ;;
    netns)
      ip link set lo up
      cat >"$artifacts_dir/dbus-session.conf" <<'XML'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
XML
      exec env LUNAR_E2E_STAGE=session \
        dbus-run-session --config-file="$artifacts_dir/dbus-session.conf" -- \
        "$script" "$@"
      ;;
    session) return 0 ;;
    *)
      echo "étape inconnue : $LUNAR_E2E_STAGE" >&2
      exit 2
      ;;
  esac
}

# Dossier d'artefacts absolu, créé, et vide (refus sinon).
prepare_artifacts_dir() {
  local prefix="$1"
  if [[ -z "$artifacts_dir" ]]; then
    artifacts_dir="$(mktemp -d "${TMPDIR:-/tmp}/$prefix.XXXXXX")"
    return 0
  fi
  mkdir -p "$artifacts_dir"
  artifacts_dir="$(cd "$artifacts_dir" && pwd)"
  if [[ -n "$(ls -A "$artifacts_dir")" ]]; then
    echo "le dossier d'artefacts n'est pas vide : $artifacts_dir" >&2
    exit 1
  fi
}

# Chemin absolu d'un fichier existant (inchangé sinon).
absolute_path() {
  if [[ -e "$1" ]]; then
    printf '%s/%s\n' "$(cd "$(dirname "$1")" && pwd)" "$(basename "$1")"
  else
    printf '%s\n' "$1"
  fi
}

# --- Session : pilote, écran, environnement, application --------------------

# Pilote XTest (script/e2e/lunar-x11-input.c), compilé dans les artefacts.
compile_input_driver() {
  local root_dir="$1" x11_cflags x11_libs
  pkg-config --exists x11 xtst ||
    fail "bibliothèques de développement X11/XTest absentes (pkg-config x11 xtst)"
  input_binary="$artifacts_dir/lunar-x11-input"
  read -r -a x11_cflags <<<"$(pkg-config --cflags x11 xtst)"
  read -r -a x11_libs <<<"$(pkg-config --libs x11 xtst)"
  cc -std=c11 -D_DEFAULT_SOURCE -O2 -Wall -Wextra -Werror "${x11_cflags[@]}" \
    "$root_dir/script/e2e/lunar-x11-input.c" -o "$input_binary" "${x11_libs[@]}" ||
    fail "compilation du pilote X11 impossible"
}

# Xvfb dédié, sans écoute TCP ni extension Composite (gpui dessine alors
# sans compositeur, dans un visuel 24 bits que lavapipe sait présenter).
start_xvfb() {
  display_is_free || fail "l'écran $display_name est occupé"
  export DISPLAY="$display_name"
  Xvfb "$DISPLAY" -screen 0 1920x1080x24 -nolisten tcp -noreset -extension Composite \
    >"$artifacts_dir/xvfb.log" 2>&1 &
  xvfb_pid=$!
  local deadline=$((SECONDS + 15))
  until xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; do
    kill -0 "$xvfb_pid" 2>/dev/null || fail "Xvfb s'est arrêté (voir xvfb.log)"
    ((SECONDS < deadline)) || fail "Xvfb ne répond pas sur $DISPLAY"
    sleep 0.2
  done
  if "$input_binary" wait-window LUNAR 300 >/dev/null 2>&1; then
    fail "une fenêtre LUNAR existe déjà sur $DISPLAY"
  fi
}

# HOME et XDG_* dans les artefacts ; GTK en X11 sans GVFS (ni
# avertissement ni délai de montage), sans dconf (réglages en mémoire) ni
# at-spi ; rendu Vulkan logiciel (lavapipe) ; filtre de journal par défaut
# de l'application (RUST_LOG ignoré).
export_isolated_env() {
  export HOME="$artifacts_dir/home"
  export XDG_DATA_HOME="$artifacts_dir/xdg-data"
  export XDG_CONFIG_HOME="$artifacts_dir/xdg-config"
  export XDG_CACHE_HOME="$artifacts_dir/xdg-cache"
  export XDG_RUNTIME_DIR="$artifacts_dir/xdg-runtime"
  mkdir -p "$HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"
  export GDK_BACKEND=x11 GDK_SCALE=1 GDK_DPI_SCALE=1 NO_AT_BRIDGE=1
  export GIO_USE_VFS=local GSETTINGS_BACKEND=memory
  export LIBGL_ALWAYS_SOFTWARE=1
  local lavapipe_icd=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
  [[ -f "$lavapipe_icd" ]] || fail "pilote Vulkan logiciel lavapipe absent : $lavapipe_icd"
  export VK_ICD_FILENAMES="$lavapipe_icd"
  unset RUST_LOG
}

# Lance le binaire (journal dans RUNTIME_LOG), attend sa fenêtre, vérifie
# qu'elle lui appartient (_NET_WM_PID) et lit sa géométrie. Les sélecteurs
# de fichiers s'ouvrent dans le dossier courant : celui des artefacts.
launch_app() {
  runtime_log="$1"
  cd "$artifacts_dir" || fail "dossier d'artefacts inaccessible"
  "$binary_path" >"$runtime_log" 2>&1 &
  app_pid=$!
  main_id="$("$input_binary" wait-window LUNAR 60000)" || fail "fenêtre LUNAR introuvable"
  local window_pid
  window_pid="$("$input_binary" pid-id "$main_id")"
  [[ "$window_pid" == "$app_pid" ]] ||
    fail "la fenêtre LUNAR n'appartient pas au processus lancé ($window_pid ≠ $app_pid)"
  focus_main
  wait_for_log_since 0 "[Cpu] Listener registered" 30
  read -r main_width main_height _ _ < <("$input_binary" geometry-id "$main_id")
}

# Attend la première image : la bande des boutons de la barre d'outils
# compte bien plus de couleurs que son fond uni. Le rendu logiciel peut
# tarder (fenêtre noire jusque-là). Écrit la capture dans PNG et rend le
# nombre de couleurs (au plus 30 s d'attente).
wait_for_first_frame() {
  local png="$1" colors=0 deadline=$((SECONDS + 30))
  while ((SECONDS < deadline)); do
    park_pointer
    import -window "$main_id" "$png"
    colors="$(convert "$png" -crop "600x$((TOOLBAR_H - 8))+0+$((MENU_BAR_H + 4))" \
      +repage -format '%k' info:)"
    ((colors >= 20)) && break
    sleep 0.5
  done
  echo "$colors"
}

stop_pid() {
  local pid="$1"
  [[ -n "$pid" ]] || return 0
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -9 "$pid" 2>/dev/null || true
  fi
  wait "$pid" 2>/dev/null || true
}

# Ferme l'application par Ctrl+Q puis Entrée (confirmation) et rend son
# code de sortie.
quit_app() {
  send_ctrl q
  sleep 1
  [[ -z "${1:-}" ]] || capture_window "$1"
  send_key Return
  local deadline=$((SECONDS + 20)) app_status=0
  while kill -0 "$app_pid" 2>/dev/null; do
    ((SECONDS < deadline)) || fail "LUNAR ne s'est pas fermé après confirmation"
    sleep 0.2
  done
  wait "$app_pid" || app_status=$?
  app_pid=""
  return "$app_status"
}

# --- Journal -----------------------------------------------------------------

# Nombre de lignes du journal : repère avant une action, pour n'attendre que
# les lignes écrites après.
log_mark() {
  if [[ -f "$runtime_log" ]]; then wc -l <"$runtime_log"; else echo 0; fi
}

# Attend PATTERN (texte exact) dans les lignes du journal après MARK.
wait_for_log_since() {
  local mark="$1" pattern="$2" timeout_seconds="${3:-30}"
  local deadline=$((SECONDS + timeout_seconds))
  while ((SECONDS < deadline)); do
    if tail -n "+$((mark + 1))" "$runtime_log" 2>/dev/null | grep -Fq -- "$pattern"; then
      return 0
    fi
    if [[ -n "$app_pid" ]] && ! kill -0 "$app_pid" 2>/dev/null; then
      fail "LUNAR s'est arrêté en attendant le log : $pattern"
    fi
    sleep 0.25
  done
  fail "log absent après ${timeout_seconds}s : $pattern"
}

# Première ligne du journal après MARK contenant PATTERN.
log_line_since() {
  tail -n "+$(($1 + 1))" "$runtime_log" | grep -F -m1 -- "$2" || true
}

wait_for_file() {
  local path="$1" timeout_seconds="${2:-20}"
  local deadline=$((SECONDS + timeout_seconds))
  while ((SECONDS < deadline)); do
    [[ -s "$path" ]] && return 0
    sleep 0.25
  done
  fail "fichier absent ou vide : $path"
}

# --- Clavier, souris, captures -----------------------------------------------

send_key() { "$input_binary" key-id "$main_id" "$1"; }
send_ctrl() { "$input_binary" ctrl-id "$main_id" "$1"; }
send_ctrl_shift() { "$input_binary" ctrl-shift-id "$main_id" "$1"; }
type_text() { "$input_binary" type-id "$main_id" "$1"; }
click_main() { "$input_binary" click-id "$main_id" "$1" "$2"; }
wheel_main() { "$input_binary" wheel-id "$main_id" "$1" "$2" "$3" "$4"; }
focus_main() { "$input_binary" focus-id "$main_id"; }

# Pointeur garé dans la partie vide de la barre de menus, à droite : pas
# d'infobulle ni de survol dans les captures.
park_pointer() {
  [[ -n "${main_width:-}" ]] || return 0
  "$input_binary" move-id "$main_id" $((main_width - 8)) $((MENU_BAR_H / 2)) 2>/dev/null || true
  sleep 0.3
}

capture_window() {
  local name="$1"
  [[ -n "$main_id" ]] || return 0
  park_pointer
  import -window "$main_id" "$artifacts_dir/$name.png"
  identify "$artifacts_dir/$name.png" >/dev/null
  log "Capture : $artifacts_dir/$name.png"
}

# Sélecteur GTK (rfd) : ouverture ou enregistrement. Ctrl+L donne le champ
# de chemin (ouverture) ou le nom (enregistrement), Ctrl+A en sélectionne le
# contenu, le chemin est collé d'un coup (paste-id : le pilote sert lui-même
# le presse-papiers ; une saisie touche par touche relance la complétion GTK
# à chaque caractère), Suppr retire une complétion en ligne éventuelle,
# Entrée valide.
fill_file_dialog() {
  local selected_path="$1" dialog_id shot
  dialog_id="$("$input_binary" wait-dialog "$main_id" 20000)" ||
    fail "sélecteur de fichiers introuvable"
  dialog_count=$((dialog_count + 1))
  shot="$artifacts_dir/dialogue-$(printf %02d "$dialog_count")"
  log "Sélecteur $dialog_count : $dialog_id ($selected_path)"
  sleep 1
  "$input_binary" focus-id "$dialog_id"
  import -window "$dialog_id" "$shot-avant.png" 2>/dev/null || true
  "$input_binary" ctrl-id "$dialog_id" l
  sleep 0.3
  "$input_binary" ctrl-id "$dialog_id" a
  "$input_binary" paste-id "$dialog_id" "$selected_path" 10000 ||
    fail "collage du chemin refusé par le sélecteur"
  sleep 0.3
  "$input_binary" key-id "$dialog_id" Delete
  "$input_binary" key-id "$dialog_id" Return
  if ! "$input_binary" wait-gone "$dialog_id" 15000 2>/dev/null; then
    import -window "$dialog_id" "$shot-apres.png" 2>/dev/null || true
    fail "sélecteur de fichiers resté ouvert ($selected_path)"
  fi
  focus_main
}

# Glisser-déposer du fichier PATH (absolu) au point (X, Y) de la fenêtre
# principale : le pilote joue la source XDND (drop-id) ; gpui transmet le
# dépôt au panneau d'import ouvert. Aucun sélecteur GTK n'est ouvert.
drop_main() {
  local x="$1" y="$2" path="$3"
  "$input_binary" drop-id "$main_id" "$x" "$y" "$path" 10000 ||
    fail "glisser-déposer refusé par la fenêtre ($path)"
  log "Déposé : $path"
}

# Clique (X, Y) jusqu'à voir PATTERN dans le journal après MARK : la vue
# peut ne pas encore montrer le bouton visé (liste de fichiers pas encore
# redessinée). Au plus trois clics, chacun suivi de 6 s d'attente.
click_until_log() {
  local x="$1" y="$2" pattern="$3" mark="$4" attempt
  for attempt in 1 2 3; do
    click_main "$x" "$y"
    for _ in $(seq 1 24); do
      if tail -n "+$((mark + 1))" "$runtime_log" | grep -Fq -- "$pattern"; then
        return 0
      fi
      sleep 0.25
    done
    log "clic ($x, $y) sans effet (essai $attempt), nouvel essai"
  done
  fail "le clic ($x, $y) n'a pas produit : $pattern"
}

# --- Géométrie de la fenêtre (un seul endroit) ------------------------------
# Les actions passent d'abord par les raccourcis clavier (actions de menu,
# déterministes). Seuls les boutons sans raccourci sont cliqués, à des
# positions tirées du code de l'interface (crates/lunar/src) et mesurées
# sur les captures (échelle 1 sous Xvfb, origine en haut à gauche de la
# fenêtre, sans gestionnaire de fenêtres) :
#   - barre de menus 28 px (widgets/menu_bar.rs), barre d'outils 40 px
#     (app/toolbar.rs) : le graphe commence à GRAPH_TOP ;
#   - en bas : journal de trames 190 px et barre de statut 22 px
#     (app/packet_log.rs, app/status_bar.rs) : la zone graphe + panneau
#     latéral finit à side_bottom ;
#   - panneaux latéraux collés à droite, largeur fixe (app/panels.rs) :
#     configuration 400, imports 480, filtre 520 px ; les abscisses sont
#     comptées depuis le bord droit (main_width) ;
#   - boutons du graphe en haut à gauche de sa zone (app/root.rs) :
#     « Export PNG » puis « Gravité » ;
#   - ordonnées des boutons d'un panneau : mesurées depuis le haut, elles ne
#     dépendent pas de la taille de la fenêtre ; « Sauvegarder » (config) et
#     « Appliquer » (filtre), en bas de panneaux qui défilent, sont visés
#     après un défilement jusqu'au bout, depuis side_bottom ;
#   - vue d'investigation : onglets et tableau à gauche, à position fixe.
{
  MENU_BAR_H=28
  TOOLBAR_H=40
  PACKET_LOG_H=190
  STATUS_BAR_H=22
  GRAPH_TOP=$((MENU_BAR_H + TOOLBAR_H))
  GRAPH_CONTROLS_Y=$((GRAPH_TOP + 20))
  GRAPH_EXPORT_X=61
  GRAPH_GRAVITY_X=164
  CONFIG_PANEL_W=400
  IMPORT_PANEL_W=480
  FILTER_PANEL_W=520
  # Import de captures ou de matrices (listes vides au départ).
  IMPORT_ADD_PCAP_Y=139
  IMPORT_ADD_MATRIX_Y=194
  IMPORT_OPEN_PCAP_Y=234   # une capture listée
  IMPORT_OPEN_MATRIX_Y=289 # une matrice listée, aucune capture
  # Import de labels (boutons pleine largeur).
  LABELS_ADD_FILE_Y=139
  LABELS_WINAUDIT_Y=177
  # Configuration : liste des interfaces ; « Sauvegarder » après défilement.
  CONFIG_SELECT_DX=180
  CONFIG_SELECT_Y=167
  CONFIG_SAVE_DX=71
  CONFIG_SAVE_DY=32
  # Filtre : case ICMP ; « Appliquer » après défilement.
  FILTER_ICMP_DX=293
  FILTER_ICMP_Y=348
  FILTER_APPLY_DX=78
  FILTER_APPLY_DY=47
  # Vue d'investigation : onglet Endpoints, première ligne (colonne Endpoint).
  INVESTIGATION_ENDPOINTS_TAB_X=234
  INVESTIGATION_TAB_Y=188
  INVESTIGATION_FIRST_ROW_X=100
  INVESTIGATION_FIRST_ROW_Y=251
}

# Abscisse du centre d'un panneau latéral de largeur W.
panel_center_x() { echo $((main_width - $1 / 2)); }
# Abscisse à DX du bord gauche d'un panneau latéral de largeur W.
panel_x() { echo $((main_width - $1 + $2)); }
# Bas de la zone graphe + panneau latéral.
side_bottom() { echo $((main_height - PACKET_LOG_H - STATUS_BAR_H)); }

# Gravité dans l'état voulu (activée|coupée) : un clic sur le bouton,
# éventuellement deux, d'après « [Graphe] gravité … ». Rend l'état final.
set_gravity() {
  local wanted="$1" mark state attempt
  for attempt in 1 2; do
    mark="$(log_mark)"
    click_main "$GRAPH_GRAVITY_X" "$GRAPH_CONTROLS_Y"
    wait_for_log_since "$mark" "[Graphe] gravité" 10
    state="$(log_line_since "$mark" "[Graphe] gravité" | sed -E 's/.*gravité ([a-zé]+).*/\1/')"
    [[ "$state" == "$wanted" ]] && break
  done
  [[ "$state" == "$wanted" ]] || fail "gravité restée $state (voulue : $wanted)"
  echo "$state"
}

# Réinitialisation du relevé (Ctrl+Shift+R, confirmée par Entrée).
reset_capture() {
  local mark
  mark="$(log_mark)"
  send_ctrl_shift r
  sleep 1
  [[ -z "${1:-}" ]] || capture_window "$1"
  send_key Return
  wait_for_log_since "$mark" "Relevé réinitialisé" 15
}

# Import d'une matrice (KIND=matrix) ou d'une capture (KIND=pcap) par le
# panneau d'import (Ctrl+O) : ajout du fichier, « Ouvrir », fin d'import
# (le panneau se ferme de lui-même). SHOT : préfixe des captures (option).
# `import_mark` reçoit le repère du journal pris avant « Ouvrir ».
# Ajout par le sélecteur GTK (« Ajouter ») ; avec `import_by_drop=1`, par
# glisser-déposer du fichier sur le panneau.
import_by_drop=""
import_file() {
  local kind="$1" path="$2" shot="${3:-}" mark add_y open_y started finished
  case "$kind" in
    matrix)
      add_y="$IMPORT_ADD_MATRIX_Y" open_y="$IMPORT_OPEN_MATRIX_Y"
      started="import_matrix_files: $path" finished="ligne(s) importée(s) ->"
      ;;
    pcap)
      add_y="$IMPORT_ADD_PCAP_Y" open_y="$IMPORT_OPEN_PCAP_Y"
      started="convert_from_pcap_list : $path" finished="FIN traitement liste PCAP"
      ;;
    *) fail "type d'import inconnu : $kind" ;;
  esac
  mark="$(log_mark)"
  send_ctrl o
  wait_for_log_since "$mark" "[Panneaux] import-captures ouvert" 10
  sleep 1
  [[ -z "$shot" ]] || capture_window "$shot-panneau"
  if [[ -n "$import_by_drop" ]]; then
    drop_main "$(panel_center_x "$IMPORT_PANEL_W")" "$add_y" "$path"
  else
    click_main "$(panel_center_x "$IMPORT_PANEL_W")" "$add_y"
    fill_file_dialog "$path"
  fi
  sleep 1
  [[ -z "$shot" ]] || capture_window "$shot-liste"
  mark="$(log_mark)"
  click_until_log "$(panel_center_x "$IMPORT_PANEL_W")" "$open_y" "$started" "$mark"
  wait_for_log_since "$mark" "$finished" 120
  wait_for_log_since "$mark" "[Panneaux] import-captures fermé" 20
  import_mark="$mark"
}
