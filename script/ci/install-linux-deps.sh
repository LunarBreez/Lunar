#!/usr/bin/env bash
# Paquets Ubuntu 22.04 nécessaires pour compiler LUNAR (gpui, GTK 3 de rfd,
# libpcap) et, avec --smoke, pour lancer le binaire sur un Xvfb avec le
# Vulkan logiciel lavapipe. Utilisé par les workflows GitHub.
set -Eeuo pipefail
packages=(
  pkg-config libpcap-dev libgtk-3-dev
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libfontconfig1-dev libfreetype-dev libx11-xcb-dev libvulkan1
)
if [[ "${1:-}" == "--smoke" ]]; then
  packages+=(xvfb mesa-vulkan-drivers)
fi
sudo apt-get update
sudo apt-get install -y --no-install-recommends "${packages[@]}"
