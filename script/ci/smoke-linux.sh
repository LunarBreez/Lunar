#!/usr/bin/env bash
# Smoke test du binaire Linux sur un Xvfb isolé avec lavapipe : --version,
# puis --lunar-smoke-test (fenêtre jusqu'à la première image, code 0 et
# LUNAR_STARTUP_VALIDATION=OK). Voir docs/gpui-plateforme.md pour Composite.
set -Eeuo pipefail
binary="${1:?usage: smoke-linux.sh <binaire>}"
display="${SMOKE_DISPLAY:-:99}"
"$binary" --version | grep -E '^lunar [0-9]'
icd=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
[[ -f "$icd" ]] || { echo "pilote lavapipe absent ($icd)" >&2; exit 1; }
Xvfb "$display" -screen 0 1920x1080x24 -nolisten tcp -extension Composite &
xvfb_pid=$!
trap 'kill "$xvfb_pid" 2>/dev/null || true; wait "$xvfb_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
  [[ -e "/tmp/.X11-unix/X${display#:}" ]] && break
  sleep 0.1
done
log="$(mktemp)"
status=0
env -u WAYLAND_DISPLAY DISPLAY="$display" VK_ICD_FILENAMES="$icd" \
  timeout 120 "$binary" --lunar-smoke-test >"$log" 2>&1 || status=$?
cat "$log"
[[ "$status" -eq 0 ]] || { echo "smoke test : code $status" >&2; exit 1; }
grep -q '^LUNAR_STARTUP_VALIDATION=OK$' "$log" || { echo "marqueur de démarrage absent" >&2; exit 1; }
echo "smoke test Linux : OK"
