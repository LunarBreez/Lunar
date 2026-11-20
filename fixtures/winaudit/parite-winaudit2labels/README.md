# Corpus de parité avec `winaudit2labels`

Relevés WinAudit **forgés** (UTF-16LE avec BOM, colonnes WinAudit) : aucune
donnée de machine réelle. `attendu-winaudit2labels.csv` est la sortie de
l'outil `winaudit2labels`, lancé **sans option** :

    winaudit2labels releves -o attendu-winaudit2labels.csv

Le test `la_sortie_est_celle_de_winaudit2labels_sur_le_corpus_de_parite`
(`lunar-flows-core/tests/winaudit_labels.rs`) exige que la conversion de LUNAR
(`lunar_flows_core::winaudit`, celle de l'import de relevés de
l'application), avec ses réglages par défaut, produise exactement ce
fichier. Si une règle dérive d'un
côté, ce test le dit.

Les deux outils partagent leurs réglages par défaut :

| | `winaudit2labels` et LUNAR |
| --- | --- |
| Interfaces virtuelles | écartées d'après leur MAC localement administrée ; `--tout` (LUNAR : la case décochée, `WinAuditOptions::keep_virtual`) les garde |
| Conflit entre relevés | le plus récent gagne (date et heure de la rubrique 300, sinon date du nom de fichier) ; à égalité, le premier lu. L'application fait en plus arbitrer l'utilisateur |

Le corpus couvre :

- l'identifiant SALP tiré du nom de fichier, et un relevé qui n'en a pas ;
- un sous-dossier de collecte ;
- le séparateur point-virgule ;
- les MAC à tirets et à points ;
- l'IPv6 en capitales avec index de zone ;
- le bouclage, l'adresse non spécifiée, une interface sans adresse ;
- un conflit entre trois relevés, dont le plus ancien est lu en dernier
  (`collecte-2/`) : c'est l'horodatage qui départage, pas l'ordre de lecture ;
- `docker0`, écartée d'après sa MAC, et « Local Area Connection », carte
  physique gardée (l'ancienne liste de préfixes de `winaudit2labels`
  l'écartait à cause de « lo »).
