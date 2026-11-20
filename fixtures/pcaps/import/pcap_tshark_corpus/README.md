# Corpus PCAP ↔ TShark multi-DLT

Ce corpus provient de la crate `packet_parser` 8.1.0 maintenue dans
`Akmot9/Packet-parser`. Il couvre les cinq types de liaison actuellement
décodés par la crate utilisée par LUNAR (Ethernet, RAW, SLL, SLL2 et, depuis la
11.2.0, le loopback BSD `NULL`), ainsi que sa frontière de support.

| Fixture | DLT | Paquets | Provenance / transformation |
|---|---:|---:|---|
| `linux_sll.pcap` | 113 | 2 702 | copie exacte de `pcaps_exemple/sll.pcap` |
| `linux_sll2.pcap` | 276 | 779 | copie exacte de `pcaps_exemple/capture_sll2.pcap` |
| `raw_ip.pcap` | 101 | 15 | ICMP IPv4/IPv6 réels, Ethernet retiré avec `editcap -C 14 -L` |
| `vlan.pcap` | 1 | 7 | trames 2163, 2166, 2168, 2176, 2177, 2184 et 2185 de `The-Ultimate-PCAP.pcapng` |
| `industrial_ethernet.pcap` | 1 | 10 | DNS, ARP, S7COMM, NTP et OpenVPN extraits de `4SICS-GeekLounge-151020.pcap` |
| `capwap_radius.pcap` | 1 | 6 | CAPWAP-Control, DTLS et RADIUS extraits de la capture `vlan0--packet-capture…cap` |
| `capwap_management.pcap` | 1 | 2 | `capwap-association-valid.pcapng` normalisé en PCAP Ethernet |
| `capwap_data.pcap` | 1 | 2 | trames réelles ToDS/FromDS embarquées dans les golden tests de `packet_parser` |
| `unsupported_ieee80211.pcapng` | 105 | 1 | beacon réel, refus explicite attendu au niveau capture |
| `loopback_null.pcap` | 0 | 1 | UDP 127.0.0.1→127.0.0.1 forgé (struct.pack) : frontière de support jusqu'à la 11.1.0, décodé depuis packet_parser 11.2.0 |
| `opcua_loopback.pcap` | 0 | 381 | session OPC UA réelle 127.0.0.1:57420 ↔ 4840 du corpus nDPI (LGPL-3.0), copie exacte de `pcaps_exemple/protocols/opcua/opcua_loopback.pcap` (SHA-256 `46eff279…`) |
| `gtpu_gn.pcap` | 1 | 32 | les 32 G-PDU non fragmentés (TCP encapsulé) de `gtp1_gn_normal_incl_fragmentation.pcap`, corpus Zeek (BSD 3-clause), extraits avec `tshark -o ip.defragment:FALSE -Y '!(ip.flags.mf==1 \|\| ip.frag_offset>0)'` |
| `gtpu_ipv6.pcap` | 1 | 2 | copie exacte de `gtp7_ipv6.pcap`, corpus Zeek (BSD 3-clause) : IPv6 encapsulé dans GTP-U |
| `asterix_cat048_multicast.pcap` | 1 | 203 | diffusion multicast ASTERIX CAT 048 du mainteneur de `packet_parser`, anonymisée avant dépôt (ports 8611/8612, hors port 8600), copie exacte de `pcaps_exemple/protocols/asterix/cat048_multicast.pcap` (SHA-256 `7c5a3c01…`) |
| `asterix_cat034_048.pcap` | 1 | 100 | trafic radar réel CAT 034 + 048 (20 datagrammes à deux data blocks), échantillon `install/sample_data/cat_034_048.pcap` du dépôt CroatiaControlLtd/asterix (GPL-2.0), copie exacte (SHA-256 `7f4e9a37…`) |
| `ptp_udp_l2.pcap` | 1 | 39 | PTPv2 Hirschmann (2007) : 25 messages sur UDP 319/320 (IPv4) et 14 directement sur Ethernet (EtherType `0x88F7`), échantillon `ptpv2.pcap` du wiki Wireshark (GPL-2.0+), copie exacte de `pcaps_exemple/protocols/ptp/ptpv2.pcap` (SHA-256 `21608642…`) |
| `ptp_ipv6.pcap` | 1 | 14 | PTPv2 Symmetricom sur UDP/IPv6 (2011), fichier `ptpv2.pcap` du corpus nDPI (LGPL-3.0), copie exacte de `pcaps_exemple/protocols/ptp/ndpi_ptpv2.pcap` (SHA-256 `e6582468…`) |
| `qinq_pppoe.pcap` | 1 | 86 | échantillon public `pppoe-over-qinq` déjà anonymisé (1.1.1.1 ↔ 2.2.2.2, MAC nulles) : double tag 802.1Q 3704/2474 devant du PPPoE session ; l'oracle et LUNAR retiennent le tag interne |
| `dnp3.pcap` | 1 | 214 | session DNP3 (IEEE 1815) réelle du corpus Zeek (BSD 3-clause), copie exacte de `pcaps_exemple/protocols/dnp3/dnp3.pcap` (SHA-256 `7a304961…`) : 115 trames DNP3, le reste en ouverture et acquittements TCP |
| `iec104.pcap` | 1 | 105 | téléconduite IEC 60870-5-104 du wiki Wireshark (GPL), copie exacte de `pcaps_exemple/protocols/iec104/wireshark_iec104.pcap` (SHA-256 `a78aa971…`) : 64 APDU |
| `bacnet_ip.pcap` | 1 | 23 | BACnet/IP du corpus nDPI (LGPL-3.0), copie exacte de `pcaps_exemple/protocols/bacnet/ndpi_bacnet.pcap` (SHA-256 `e64557f7…`) |
| `goose_l2.pcap` | 1 | 8 | GOOSE (IEC 61850-8-1) d'un relais GE F650, directement sur Ethernet (EtherType `0x88B8`), corpus ITI/ICS-Security-Tools (CC-BY-4.0), copie exacte de `pcaps_exemple/protocols/goose/iti_GOOSE.pcap` (SHA-256 `746619df…`) |
| `radius.pcap` | 0 | 19 | RADIUS sur loopback BSD, corpus Zeek (BSD 3-clause), copie exacte de `pcaps_exemple/protocols/radius/radius_localhost.pcap` (SHA-256 `4822c70d…`) |
| `ike_v1_main_mode.pcap` | 1 | 12 | négociation IKEv1 en mode principal du wiki Wireshark (GPL), copie exacte de `pcaps_exemple/protocols/ipsec/attachment_ikev1_main_mode.pcap` (SHA-256 `5c75e96c…`) |
| `wireguard.pcap` | 1 | 22 | tunnel WireGuard (initiation, réponse, données) du dépôt Wireshark (GPL-2.0-or-later), copie exacte de `pcaps_exemple/protocols/wireguard/wireshark_wireguard-ping-tcp.pcap` (SHA-256 `936b0dd7…`) |
| `rtps_cyclonedds.pcap` | 0 | 87 | RTPS (DDS) Eclipse Cyclone DDS sur loopback BSD, dépôt foxglove/rtps (MIT), `talker-listener-rtps.pcapng` réécrit en PCAP avec `editcap -F pcap` |
| `dis.pcap` | 1 | 253 | PDU DIS (IEEE 1278.1) Signal et Transmitter du wiki Wireshark (GPL), `DIS_signal_and_transmitter.pcapng` réécrit en PCAP avec `editcap -F pcap` |
| `stanag5066_expedited.pcap` | 1 | 13 | primitives STANAG 5066 SIS (TCP 5066) du wiki Wireshark (GPL), copie exacte de `pcaps_exemple/protocols/stanag5066/S5066-Expedited.pcap` (SHA-256 `dd5349fd…`) |

Les captures ICMP et 802.11 proviennent du corpus de Chris Sanders
(`<https://github.com/chrissanders/packets>`), conformément aux fichiers
`SOURCE.md` de `Packet-parser`. Les captures complètes SLL/SLL2 sont des
captures locales `tcpdump -i any` datées du 14 juillet 2026. Elles ne sont
pas anonymisées.

## Audit de sensibilité (07/08/2026)

Inventaire exact de ce que `linux_sll.pcap` et `linux_sll2.pcap` exposent :

- le préfixe IPv6 public du réseau de capture (`2001:861:3fc7:9b00::/64`,
  Free), avec des identifiants d'interface EUI-64 dérivés de MACs réelles ;
- les adresses MAC des équipements du réseau de capture ;
- des requêtes DNS et SNI TLS vers des services publics
  (`accounts.google.com`, `unagi.amazon.fr`, `unleash.codeium.com`, entre
  autres) — des métadonnées de navigation, pas de contenu.

**Décision du propriétaire (07/08/2026) : exposition documentée et
acceptée.** Il s'agit de son propre réseau domestique et de ses propres
métadonnées, publiées en connaissance de cause pour la valeur de test des
captures cooked réelles. Le reste du corpus provient de corpus publics déjà
publiés (nDPI, Chris Sanders, 4SICS, packet_parser) ou de fichiers forgés
(`loopback_null.pcap`), sans donnée sensible.

## Oracles indépendants

Chaque `*.flows.tsv` est généré par TShark 4.6.6 avec résolution de noms
désactivée. La clé directionnelle compare les adresses de liaison disponibles,
VLAN, EtherType, IP, protocole IP et ports. Les statistiques comparent le
nombre de paquets, les octets sur le fil et le dernier timestamp.

Pour GTP-U, `*.flows.tsv` décrit le flux externe UDP 2152 et
`*.inner.flows.tsv` le paquet IP transporté par chaque G-PDU (projection
`--gtpu-inner`), sans MAC, avec pour taille celle du datagramme interne. Les
captures GTP-U viennent du corpus de tests de Zeek, sous licence BSD 3-clause :
son texte complet, `LICENSE-ZEEK.txt`, les accompagne comme la licence
l'exige. Seuls les datagrammes externes non fragmentés sont retenus, TShark
réassemblant les fragments IP que packet_parser ne réassemble pas.

Pour ASTERIX, les oracles ne décrivent que les flux UDP : TShark
ne reconnaît ASTERIX que sur décision explicite (`-d udp.port==…,asterix`),
tandis que packet_parser le reconnaît par sa seule structure. Le test Rust
vérifie donc à part que chaque flux de ces deux captures porte l'étiquette
`ASTERIX`, soit exactement les 303 trames que `tshark -Y asterix` voit. La
capture CroatiaControlLtd est redistribuée telle quelle sous GPL-2.0 : son
texte complet, `LICENSE-CROATIACONTROL-GPL-2.txt`, l'accompagne.

Pour PTP (packet_parser 11.5.0), les oracles décrivent les flux UDP et les
trames de couche 2 (EtherType `0x88F7`, sans IP). Le test Rust vérifie à part
que les 39 messages UDP (25 + 14) portent l'étiquette `PTP` et que les 14
trames de couche 2 ont `PTP` pour protocole de liaison : exactement les comptes
de `tshark -Y 'ptp && udp'` et `tshark -Y 'ptp && !udp'`. L'aller-retour LFMS
de ces trames exerce le réimport de la nouvelle liaison. Licences jointes :
`LICENSE-WIRESHARK-WIKI-GPL-2.txt` pour l'échantillon du wiki Wireshark, et
`LICENSE-NDPI-LGPL-3.txt` avec `LICENSE-NDPI-GPL-3.txt` (la LGPL-3.0 complète
la GPL-3.0) pour les captures du corpus nDPI, `opcua_loopback.pcap` comprise.

Pour les décodeurs de packet_parser 11.6.0 (DNP3, IEC 104, BACnet/IP, GOOSE,
RADIUS, IKE, WireGuard, RTPS, DIS, STANAG 5066), les oracles décrivent les flux
sans étiquette ; le test Rust `packet_parser_11_6_captures_are_labelled…` fige
à part que chaque capture porte exactement les trames que le filtre TShark du
protocole voit (`dnp3`, `iec60870_104`, `bvlc`, `radius`, `isakmp`, `wg`,
`rtps`, `dis`, `s5066sis`) et aucune autre étiquette, et que les 8 trames
GOOSE ont `GOOSE` pour protocole de liaison (`tshark -Y goose`). Licences
jointes : `LICENSE-ZEEK.txt` (DNP3, RADIUS), `LICENSE-WIRESHARK-WIKI-GPL-2.txt`
(IEC 104, IKE, DIS, STANAG 5066, et WireGuard du dépôt Wireshark, même texte
GPL-2), `LICENSE-NDPI-LGPL-3.txt` (BACnet/IP), `LICENSE-ITI-CC-BY-4.0.txt`
(GOOSE, attribution ITI/ICS-Security-Tools) et `LICENSE-FOXGLOVE-MIT.txt`
(RTPS). Depuis cette version, le générateur d'oracle pèle aussi la session
PPPoE comme un tag VLAN quand PPP porte IPv4 ou IPv6 (`ppp.protocol`), comme
le parseur : `qinq_pppoe.flows.tsv` décrit désormais les flux IP transportés.

Pour CAPWAP, `capwap_data.flows.tsv` décrit le flux Ethernet/IP/UDP externe et
`capwap_data.inner.flows.tsv` décrit le niveau 802.11/LLC/IP/TCP interne. Les
tests imposent en plus un même `encap_id` hexadécimal aux deux niveaux et aux
deux sens. La capture d'association contient des trames de gestion 802.11 :
elle doit rester un flux CAPWAP externe sans inventer de conversation interne.

Chaque oracle contient la version TShark, le SHA-256 de sa capture, son domaine
de projection et ses totaux. Le test Rust recalcule le SHA-256 hors ligne avant
toute comparaison afin qu'une capture modifiée rende immédiatement l'oracle
périmé.

Les oracles ont été produits par un générateur TShark
(`generate-common-flows.py`, nommé dans l'en-tête de chaque oracle) qui n'est
pas inclus dans ce dépôt. La règle reste stricte : un nouveau DLT doit
recevoir une projection explicite dans le générateur ; il n'est jamais
assimilé silencieusement à Ethernet.
