# Relevés WinAudit forgés

Relevés de configuration WinAudit **forgés** : aucune donnée de machine
réelle. Même encodage que le collecteur (UTF-16LE avec BOM) et mêmes
positions de colonnes que les relevés réels : rubrique `300` (nom de
machine en colonne 2), rubrique `2600` (adresses en colonne 6, type en
colonne 16, MAC en colonne 17).

## Parcours E2E X11

`20260924_NP_CONF_TST-PC-000001.csv` suit la convention de nommage des
relevés (`AAAAMMJJ_CLASSIF_CONF_IDENTIFIANT.csv`) : son label est « TST-PC-000001 POSTE-E2E-WINAUDIT » (identifiant
SALP du nom de fichier, puis nom de machine). Le parcours E2E X11
(`script/e2e/run-lunar-x11-e2e.sh`) l'importe après la capture
`pcaps/import/pcap_tshark_corpus/industrial_ethernet.pcap` et le fichier de
labels `20260703_NP_Labels.csv` :

| Adaptateur | MAC / IP | Effet attendu |
| --- | --- | --- |
| Connexion au réseau local | `28:CF:E9:18:B5:ED` / `192.168.88.52`, un poste de la capture | adresse et ligne de repli à IP vide **ajoutées** |
| Connexion au réseau local 2 | `44:15:24:20:A5:64` / `192.168.1.254`, déjà nommé « Passerelle » par le fichier de labels | **désaccord** : l'arbitrage s'ouvre, « Passerelle » n'est pas écrasé ; la ligne de repli de cette MAC s'ajoute |
| `lo` | `00:00:00:00:00:00` / `127.0.0.1`, type `Software Loopback` | bouclage **écarté** |

Bilan attendu : 3 labels ajoutés, 1 conflit.

## Parité avec `winaudit2labels`

`parite-winaudit2labels/` : corpus et sortie de référence de l'outil
`winaudit2labels`. Voir son README.
