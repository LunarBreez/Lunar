#!/usr/bin/env bash
# Fuzzing borné : prépare le corpus, puis donne à chaque cible un budget en
# secondes (90 par défaut, ou FUZZ_SECONDS). C'est le contrôle de la CI, qui
# se lance tel quel en local ; un crash laisse son entrée dans
# crates/lunar-flows-core/fuzz/artifacts/<cible>/.
# Usage : smoke.sh [cible…] (toutes les cibles par défaut). Exige cargo-fuzz
# et la toolchain nightly du crate (script/ci/install-cargo-fuzz.sh).
set -Eeuo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
seconds="${FUZZ_SECONDS:-90}"

"$root/script/fuzz/prepare-corpus.sh"

# Depuis le répertoire du crate, rustup applique sa toolchain nightly.
cd "$root/crates/lunar-flows-core/fuzz"

# Cible de l'hôte, donnée explicitement. Par défaut cargo-fuzz vise la cible
# pour laquelle il a lui-même été compilé ; son binaire officiel pour Linux
# l'est pour musl, cible absente de la toolchain et refusée par les
# sanitizers (libc statique).
host="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$host" ]] || { echo "cible de l'hôte introuvable (rustc -vV)" >&2; exit 1; }

targets=("$@")
if ((${#targets[@]} == 0)); then
  # Affectation simple : un échec de `cargo fuzz list` arrête le script, au
  # lieu de laisser une liste vide passer pour « aucun crash ».
  listed="$(cargo fuzz list)"
  mapfile -t targets <<<"$listed"
fi
if ((${#targets[@]} == 0)) || [[ -z "${targets[0]}" ]]; then
  echo "aucune cible de fuzzing à lancer" >&2
  exit 1
fi

for target in "${targets[@]}"; do
  echo "=== $target : $seconds s ==="
  cargo fuzz run --target "$host" "$target" -- \
    -max_total_time="$seconds" -rss_limit_mb=4096 -timeout=10
done
echo "Fuzzing borné : ${#targets[@]} cibles, aucun crash."
