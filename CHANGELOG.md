# Changelog

Les versions suivent le format `## **[X.Y.Z] - AAAA-MM-JJ**`. L'entrée
« Changelog » du menu Aide affiche la plus récente.

## Non publié

- **Protocoles** : ModbusUDP, Java RMI et SIP sont reconnus (packet_parser
  13.2.0) dans la matrice, le graphe, la synthèse et le glossaire.
- **Matrices hostiles** : les comptes de paquets saturent au lieu de déborder
  quand un fichier importé annonce des totaux au-delà de la capacité d'un
  compteur.
- **Changelog** : l'entrée « Changelog » du menu Aide affiche les notes de la
  dernière version au lieu de « Fonction à venir ».
- **Fuzzing** : cibles cargo-fuzz sur les lecteurs de fichiers non fiables
  (capture PCAP, matrice CSV, relevé WinAudit) et sur le décodage des trames
  reçues en capture directe, bornées en CI.
- **Release** : contrôle de passivité du binaire Linux (aucune primitive
  d'émission libpcap, aucun client HTTP embarqué).
- **Release** : plancher glibc du binaire Linux, qui ne doit rien exiger
  au-delà de la glibc 2.35 d'Ubuntu 22.04.
- **Release** : le binaire Windows est contrôlé pour charger `wpcap.dll` en
  différé, condition de son démarrage sans Npcap.
- **Sécurité** : `url` 2.5.9 corrige un débordement de pile sur des URL
  `blob:` imbriquées (RUSTSEC-2026-0592) ; le crate est tiré par gpui.
- **Dépendances** : sources vendorisées dans `vendor/`, limitées aux cibles
  livrées (Linux et Windows x86_64) ; toute compilation se fait hors ligne.
- **Sécurité** : politique de signalement des vulnérabilités (`SECURITY.md`).
- **Logo** : croissant bleu marine sur fond blanc, en SVG.

## **[0.1.0] - 2026-11-20**

Première version de LUNAR : relevé de matrices de flux réseau, avec une
interface native gpui.

- **Capture en direct** : écoute passive d'une interface, filtre BPF avec
  préréglages, bilan de capture.
- **Import** : captures PCAP et PCAPNG, matrices LFMS en CSV ou XLSX, fusion
  de plusieurs fichiers d'un même type de liaison.
- **Graphe des flux** : placement ForceAtlas2 déterministe, tunnels, légende,
  export PNG.
- **Labels** : fichiers CSV, relevés WinAudit, détection et arbitrage des
  conflits.
- **Investigation** : indicateurs, résumé par volume, tableaux des flux et des
  équipements, déductions, synthèse HTML.
- **Projets et exports** : archives `.lunar`, sauvegarde automatique et
  récupération, export de la matrice CSV, des labels et des journaux.
- **Livraison** : binaires Linux et Windows x86_64, SBOM CycloneDX,
  attestation de provenance et signature Sigstore.
