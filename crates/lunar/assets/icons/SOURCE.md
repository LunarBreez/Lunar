# Provenance des icônes

## Tabler Icons (licence MIT, voir `LICENSE-tabler.txt`)

- Paquet npm : `@tabler/icons`, version **3.50.0** (publiée le 27/10/2026).
- Archive : `https://registry.npmjs.org/@tabler/icons/-/icons-3.50.0.tgz`,
  téléchargée le 19/11/2026.
- Intégrité vérifiée contre les métadonnées du registre :
  - `dist.integrity` :
    `sha512-HflLxyrH1cqC7s7jzQGSjpsTAVIoaf7+/yHjbdM59rJQBMjliibp+LYhDPFxOMcwa1M8DNpOk6ENzQiWn9a1Dg==`
  - `dist.shasum` (SHA-1) : `fb2bdeec8896e173c34789902aeb51ffda139753`
  - SHA-256 de l'archive : `4c2eb678ed71cdcbf57a3e899c4e91f3445d680fbc1d57d27c7083a321c47303`
- Fichiers copiés sans modification depuis `package/icons/outline/` : tous les
  `*.svg` de ce répertoire sauf les deux fichiers ci-dessous.
- `TablerCpu.svg` (sélecteur d'interface) : géométrie de l'icône `cpu` de
  Tabler Icons (`package/icons/outline/cpu.svg`) mise à l'échelle sur une
  grille de 32 × 32 (facteur 4/3), trait blanc de 2,67 px.

## Roue dentée

- `mdi--gear.svg` (bouton Configuration) : le nom suit la convention Iconify
  `<collection>--<icône>` pour l'icône `gear` de Material Design Icons. Le
  fichier, dessiné sur une grille de 100 × 100, ne porte ni métadonnées ni
  mention de licence.

## LUNAR

- `../images/logo-256.png` : logo de LUNAR (croissant de lune aux couleurs du
  thème), dessiné le 19/11/2026 avec ImageMagick (`convert`, cercles et
  rectangle arrondi sur une toile de 1024 px, réduite à 256 px).
- `../images/logo.svg` : le même logo en vectoriel, redessiné à la main le
  20/11/2026 d'après le PNG (deux arcs de cercle, trois disques, rectangle
  arrondi). Il sert au README ; l'application embarque toujours le PNG.
