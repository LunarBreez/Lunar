#!/usr/bin/env bash
# Compilation Windows de contrôle depuis Linux, avec cargo-xwin : clippy sur
# toute la cible x86_64-pc-windows-msvc, puis lunar.exe en profil
# windows-check (voir Cargo.toml). Jamais pour une livraison : binaire non
# signé, console visible, shaders compilés au lancement.
#
#   script/build-windows-check.sh
#
# Prérequis : cargo-xwin, cible x86_64-pc-windows-msvc sur la toolchain
# 1.97.1, llvm-readobj (rustup component add llvm-tools). Le SDK Npcap est
# dans windows/npcap-sdk.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export RUSTUP_TOOLCHAIN=1.97.1
# Le build.rs du crate pcap tourne sur l'hôte Linux et ne peut pas sonder
# wpcap.dll : niveau d'API imposé, celui que garantit Npcap.
export LIBPCAP_LIBDIR="$ROOT/windows/npcap-sdk/Lib/x64"
export LIBPCAP_VER=1.0.0
TARGET=x86_64-pc-windows-msvc

cargo xwin clippy --workspace --all-targets --locked --target "$TARGET" -- -D warnings
cargo xwin build --profile windows-check --locked --target "$TARGET" -p lunar --bin lunar
EXE="target/$TARGET/windows-check/lunar.exe"

# wpcap.dll doit rester en chargement différé : sinon Windows refuse de
# lancer LUNAR quand Npcap n'a pas le mode WinPcap.
script/ci/check-windows-npcap-delayload.sh "$EXE"
ls -l "$EXE"
