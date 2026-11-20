# LFMS — LUNAR Flow Matrix Standard (v1)

> Description du format des matrices de flux produites et consommées par
> LUNAR. Ce document décrit le format **tel qu'implémenté** dans
> `lunar-flows-core` (module `lfms` pour le préambule, `csv` et `matrix`
> pour les lignes).

## Principes

- **Une ligne = une conversation observée**, au niveau paquet : rien n'est
  agrégé, inféré ou interprété au-delà de ce qui a été vu sur le réseau.
- **Un relevé = un réseau = un type de liaison (DLT)** : une matrice porte
  le DLT de sa source ; la fusion de sources de DLT différents est refusée
  explicitement.
- **Déterminisme** : deux exports de la même matrice sont identiques octet
  pour octet. Le préambule ne porte volontairement pas de date d'export.
- **Aller-retour sans perte** : une matrice exportée puis réimportée
  reconstruit exactement les mêmes conversations (identité LFMS préservée,
  y compris pour RAW, NULL, SLL et SLL2, et pour les niveaux internes sans
  trame des tunnels IP).

## Préambule

La première ligne d'un export CSV est une ligne de métadonnées :

```
#LFMS version=2 dlt=ETHERNET site=Usine%20Nord sensor=sonde-01 interface=eth0
```

Un export sans contexte de relevé reste exactement l'ancienne ligne, à la
version près :

```
#LFMS version=2 dlt=ETHERNET
```

- `version` : version du format de préambule — **`2`** à l'écriture. La
  version `1` reste lisible : elle ne portait pas les clés de contexte
  ci-dessous.
- `dlt` : type de liaison du relevé — mnémonique `LINKTYPE_*` connu
  (`ETHERNET`, `NULL`, `RAW`, `LINUX_SLL`, `LINUX_SLL2`, `IEEE802_11`) ou valeur
  numérique tcpdump pour les autres. Jamais de nom inventé.
- `site`, `sensor`, `interface` : contexte du relevé, ajoutés en version 2.
  **Une clé absente n'est pas émise** — un champ non renseigné ne produit
  pas `site=` vide. Les valeurs sont percent-encodées, l'espace devenant
  `%20`, pour qu'un nom de site puisse en contenir sans casser le découpage
  du préambule.
- `first_seen`, `last_seen` : période observée du relevé,
  horodatages du premier et du dernier paquet vus, en RFC 3339 UTC
  à la microseconde (`first_seen=2026-10-12T08:30:00.123456Z`). Les deux
  clés vont ensemble ; une seule, une date illisible ou des bornes inversées
  rendent le préambule illisible. Elles sont émises seulement si la période
  est connue : relevé capté ou importé depuis un PCAP, ou fusion de matrices
  qui la déclarent toutes. Au réimport, les périodes des fichiers sont
  réunies ; un fichier qui porte des lignes sans période (export antérieur,
  classeur XLSX) rend celle du relevé inconnue, et elle n'est plus écrite.
  Tirées des seules données, elles ne cassent pas le déterminisme : deux
  exports de la même matrice restent identiques octet pour octet. Un lecteur
  qui ne les connaît pas les ignore (clés inconnues tolérées).
- Les clés inconnues sont tolérées à l'import (extensions futures) ; un
  préambule présent mais illisible est une erreur, pas un avertissement.
- Un fichier **sans** préambule (export antérieur au format) est importé
  avec un DLT implicite Ethernet.

### Cellules commençant par un caractère de formule

Toute cellule de **label** dont la valeur commence par `=`, `+`, `-` ou `@`
est préfixée d'une apostrophe à l'écriture, convention tableur qui empêche
l'interprétation comme formule. L'apostrophe est retirée au réimport :
l'aller-retour ne modifie pas la donnée.

Les autres colonnes textuelles — dont `origin`, qui reçoit le nom d'un
fichier importé et n'est donc pas sous le contrôle de l'application — ne
sont pas encore neutralisées. Tant que la neutralisation ne couvre pas
l'ensemble des colonnes, ouvrir un export dans un tableur appelle la prudence habituelle
face à un CSV d'origine externe.

## Colonnes

| Colonne | Type | Sémantique |
|---|---|---|
| `mac_source` | chaîne | Adresse MAC source de la conversation. |
| `mac_destination` | chaîne | Adresse MAC destination. |
| `vlan_id` | entier ou vide | Identifiant 802.1Q si observé. Sur une trame à double tag (802.1ad / QinQ), c'est le **tag interne** : la pile complète n'est pas sérialisée. |
| `protocol_data_link` | chaîne | Protocole de couche 2 (ex. `Ipv4`, `Arp`). |
| `ip_source` | IP ou vide | IP source ; vide pour un flux sans couche 3. |
| `ip_source_type` | chaîne | Qualification de l'adresse : `Private`, `Public`, `Multicast`, `Loopback`, `Apipa`, `LinkLocal`, `Ula`, `Documentation`, `Unknown`. |
| `label_source` | chaîne ou vide | Label de l'équipement source (référentiel de labels). |
| `ip_destination` | IP ou vide | IP destination. |
| `ip_destination_type` | chaîne | Même qualification que `ip_source_type`. |
| `label_destination` | chaîne ou vide | Label de l'équipement destination. |
| `port_source` | entier ou vide | Port source (transport). |
| `port_destination` | entier ou vide | Port destination. |
| `protocol_transport` | chaîne ou vide | Protocole de transport (`TCP`, `UDP`, …). |
| `application_protocol` | chaîne ou vide | Protocole applicatif reconnu par le parsing (dont OT/ICS : Modbus, Profinet, OPC UA, S7Comm, SNMP, EtherNet/IP…). |
| `count` | entier | Nombre de paquets observés pour cette conversation. |
| `total_bytes` | entier | Total des octets observés. |
| `last_seen` | date | Dernière observation, format `AAAA-MM-JJ HH:MM:SS[.µµµµµµ] UTC`. |
| `encap_id` | chaîne ou vide | Tunnels traversés (extension LFMS, voir ci-dessous). |
| `origin` | chaîne ou vide | Provenance de la ligne (voir ci-dessous). |

Les colonnes `encap_id` et `origin` sont optionnelles à l'import
(compatibilité avec les matrices exportées avant leur introduction).

### `encap_id` — tunnels (extension LFMS)

- vide : flux jamais observé dans un tunnel ;
- `id` (16 caractères hexadécimaux) : un seul tunnel portant tous les
  paquets de la ligne ;
- `id:n|id:n|…` : comptes ventilés par tunnel quand un flux apparaît dans
  plusieurs tunnels ou partiellement hors tunnel.

L'identifiant est partagé entre la ligne externe (le tunnel, ex. CAPWAP) et
ses lignes internes (le trafic décapsulé), dans les deux sens. La somme des
`n` d'un tunnel sur les lignes filles égale le compteur de sa ligne externe.

### `origin` — provenance

Noms des fichiers ayant observé la conversation, joints par `|` quand la
ligne résulte d'une fusion multi-fichiers (ex. `site-a.csv|site-b.csv`).
Vide pour une capture live ou un import PCAP direct. Limite connue : un nom
de fichier contenant `|` est ambigu au réimport.

## Règles d'import

- **Validation stricte** : une IP invalide ou une date illisible **rejette
  la ligne avec un message précis** — jamais de dégradation silencieuse.
  L'import refusé ne mute pas le relevé en cours.
- **CSV** : séparateur virgule, en-têtes LFMS obligatoires.
- **XLSX** : première feuille lue avec le même schéma et la même validation
  que le CSV ; les colonnes supplémentaires (même insérées au milieu) sont
  ignorées tant que les en-têtes LFMS sont conservés ; les dates Excel sont
  normalisées au format LFMS.
- **Fusion** : l'import multi-fichiers fusionne les conversations
  identiques (compteurs additionnés, `origin` cumulé) ; la fusion
  inter-DLT est refusée.

## Protection tableur

Les cellules commençant par un caractère de formule (`=`, `+`, `-`, `@`)
sont préfixées par `'` à l'export (convention tableur, contre l'injection de
formule CSV). Le préfixe est retiré au réimport : l'aller-retour est
préservé.
