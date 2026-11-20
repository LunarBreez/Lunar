# Notes de plateforme : gpui 0.2.2 (19/11/2026)

Prototype : `crates/lunar/examples/gpui_spike.rs` (fenêtre « LUNAR », N nœuds et
M arêtes animés dans un `canvas`, dialogue rfd GTK). Options : `--nodes`,
`--edges`, `--frames N` (mesure puis sortie), `--dialog`. Construction :
`cargo build --release -p lunar --example gpui_spike`. Les
contournements de plateforme sont dans `src/platform.rs`.

## Résultats

| Environnement | Graphe | Images/s | Pire image |
|---|---|---|---|
| XWayland, poste (Intel UHD + Quadro T1000) | 5 000 nœuds, 10 000 arêtes | 24 à 26 | 50 ms |
| Xvfb + lavapipe (rendu logiciel) | 50 / 50 | 20 | — |
| Xvfb + lavapipe | 1 000 / 2 000 | 7 | 208 ms |
| Xvfb + lavapipe | 5 000 / 10 000 | 2 | 560 ms |

Le dialogue de fichiers GTK (rfd 0.16, backend `gtk3`) s'ouvre depuis gpui,
en français, avec le filtre PCAP.

## Contournements nécessaires

1. **Compilation** : `xattr` 0.2.3 (tiré par `gpui_http_client`) ne compile
   plus avec `libc` ≥ 0.2.191 (`ENOATTR` retiré). `Cargo.lock` fige
   `libc` 0.2.186, `tokio` 1.53.3 et `mio` 1.2.1.
2. **Xvfb sans gestionnaire de fenêtres** : gpui ne lit la socket X que
   lorsqu'elle devient lisible. Les événements lus pendant l'ouverture
   (MapNotify, Expose) restent dans la file XCB et la boucle de
   rafraîchissement ne démarre jamais. Un thread du pilote Vulkan lit aussi
   la connexion. Remède : `x11_wakeup()`, un PropertyNotify envoyé par une
   seconde connexion toutes les 100 ms jusqu'au deuxième rendu, pendant
   2 min au plus (12/12 lancements réussis). En rendu logiciel à froid, la
   première image peut prendre plusieurs secondes : une limite de 10 s ne
   suffisait pas sur un runner GitHub à 4 cœurs. gpui 0.2.2 n'implémente pas `window_handle()` sous
   X11 : la fenêtre est retrouvée par `_NET_WM_PID`.
3. **Xvfb et lavapipe** : rien n'est affiché dans une fenêtre au visuel
   32 bits (ARGB), que gpui choisit quand Composite est présent. Lancer
   `Xvfb … -extension Composite` pour les essais isolés.
4. **Wayland natif** : le backend Wayland de gpui 0.2.2 ne présente plus
   d'image après la première sur un graphe lourd. `force_x11_backend()`
   retire `WAYLAND_DISPLAY` au démarrage quand `DISPLAY` existe (XWayland).
5. **Dialogues GTK en essai isolé** : comme le harnais E2E (`script/e2e`),
   lancer sous `dbus-run-session` avec `GDK_BACKEND=x11`.

## Recette d'essai isolé

```bash
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u XAUTHORITY \
  dbus-run-session -- bash -c '
    Xvfb :91 -screen 0 1920x1080x24 -nolisten tcp -extension Composite &
    export DISPLAY=:91 GDK_BACKEND=x11 \
      VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
    ./target/release/examples/gpui_spike --nodes 50 --edges 50 --frames 60'
```

Captures : `import -window <id>` (l'identifiant via `xwininfo -root -tree`),
pas `-window root`.
