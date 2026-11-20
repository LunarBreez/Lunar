# Ultimate Ethernet sample

Cette fixture est un échantillon reproductible de
`The-Ultimate-PCAP.pcapng`. Le fichier source ne peut pas être donné tel quel
à `libpcap` : ses 313 interfaces mélangent plusieurs encapsulations et
plusieurs `snaplen`. Sur Linux/libpcap 1.10, l'ouverture échoue explicitement
avec `an interface has a snapshot length 8192 different from the snapshot
length of the first interface`.

## Provenance

- source SHA-256 :
  `ecbca543fbe011a85c8876420fcb2c244b9290ad653d13cd1a574158167073f4` ;
- source : 51 328 trames, dont 39 557 Ethernet, 10 667 IEEE 802.3br
  mPackets, 1 067 Linux SLL et 37 PPP ;
- oracle : TShark 4.6.6, résolution de noms désactivée, fuseau UTC ;
- échantillonnage : les trois premières trames Ethernet de chaque valeur
  distincte de `_ws.col.Protocol`, dans l'ordre du fichier source ;
- normalisation : écriture PCAPNG filtrée, puis `editcap -F pcap -T ether` ;
- fixture SHA-256 :
  `91252b10568ae9faed0c7262aa16c7c824643d478a392689345e1f63a43f0dde`.

La fixture résultante couvre 112 libellés de protocole TShark avec 328 trames
et 96 622 octets. Le rapport `ultimate_ethernet_sample.tshark.md` contient
l'inventaire paquet par paquet, la hiérarchie des protocoles, les conversations
et les endpoints. Sa vérification agrégée donne **PASS** face au snapshot LFMS :
220 lignes externes, 328 paquets et 96 622 octets. Les 6 lignes internes de
tunnel (GRE-dans-GRE, IPv6 dans GRE, 6in4), jointes à leur ligne externe par
`encap_id`, décrivent une seconde fois les mêmes trames et ne sont pas
totalisées.

Le test Rust compare ensuite les 226 lignes LFMS complètes et vérifie un second
export octet par octet. TShark et LUNAR n'ont pas la même taxonomie applicative ;
le CSV fige donc aussi la projection LFMS détaillée attendue.
Depuis packet_parser 11.4.0, les réponses RIPv2 des trames 69 et 75 (UDP 520
vers 224.0.0.9) ne sont plus étiquetées `DNS` par la sonde aveugle, mais
`Unknown` : LUNAR n'a pas de libellé RIP, et TShark y voit bien du RIPv2.
Depuis la 11.5.0, les adresses `2001:db8::1` et `2001:db8::2` (préfixe de
documentation IPv6, RFC 3849) sont classées `Documentation` et non plus
`Publique`, comme leurs pendantes IPv4.
Depuis la 11.7.0 (montée en 11.8.2), les trames 188 à 190 (deux
requêtes et une réponse OCSP sur TCP 80) sont étiquetées `HTTP` et non plus
`Unknown` : la sonde reconnaît les réponses et les requêtes à corps binaire.
TShark y voit de l'OCSP, que HTTP transporte.
Le préambule du snapshot porte la période observée
(`first_seen=2009-05-04T07:34:16.122475Z last_seen=2026-05-23T14:38:36.109585Z`),
identique aux premier et dernier paquets que donne `capinfos -a -e`.

## Comparaison différentielle TShark ↔ LFMS

`ultimate_ethernet_sample.flows.tsv` est un second oracle, indépendant du CSV
LUNAR. Il est généré directement depuis les champs numériques TShark par le
générateur d'oracles (`generate-common-flows.py`, hors de ce dépôt), appliqué
à `ultimate_ethernet_sample.pcap`.

La clé directionnelle commune compare les MAC, le VLAN, l'EtherType, les IP,
le numéro de protocole IP et les ports. Les statistiques comparent le nombre de
trames, la somme de `frame.len` et le dernier timestamp à la microseconde. Les
lignes LUNAR qui ne diffèrent que par le protocole applicatif sont fusionnées
avant comparaison.

L'oracle couvre 313 trames, 94 091 octets et 202 flux, tous égaux à la matrice.
Quinze trames sont hors du domaine commun : longueurs IEEE 802.3 et VLAN dont
le payload EtherType n'est pas représentable sans perte dans LFMS v1. Cette
exclusion est effectuée par des règles structurelles explicites dans le
générateur, jamais à partir du résultat LUNAR.

### Niveaux internes des tunnels IP

`ultimate_ethernet_sample.inner.flows.tsv` projette, pour les seules trames
dont le transport externe est un tunnel IP (IP-in-IP, 6in4, GRE), le flux IP
le plus interne vu par TShark — sans MAC ni VLAN, avec la taille du datagramme
interne. Il fige les lignes internes que LUNAR produit depuis packet_parser
10.4.0 ; le même générateur le produit avec l'option `--ip-inner`.
