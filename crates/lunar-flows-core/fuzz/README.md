# Fuzzing de `lunar-flows-core`

Les cibles injectent des octets arbitraires dans les lecteurs de fichiers non
fiables et dans le décodage des trames, par la même chaîne que l'application. Un fichier refusé est un
résultat normal ; un panic, un crash, une erreur de sanitizer ou un blocage
est un défaut.

| Cible | Chaîne exercée |
| --- | --- |
| `pcap_reader` | Ouverture libpcap, contrôle du type de liaison, lecture exhaustive, `packet_parser`, agrégation dans `FlowMatrix`, puis lignes à plat et graphe. |
| `matrix_reader` | Préambule `#LFMS`, lignes CSV validées champ par champ, reconstruction des flux et de leurs tunnels, fusion, labels, puis lignes à plat et graphe. |
| `winaudit_reader` | Décodage (UTF-16, UTF-8), détection du séparateur, rubriques lues par position, filtrage des adaptateurs, arbitrage entre relevés. |
| `packet_frames` | Une trame brute et son type de liaison, sans libpcap ni format de fichier : décodeurs de `packet_parser`, puis dépliage des tunnels en niveaux de flux, comme en capture directe. |

| `xlsx_reader` | Classeur XLSX : conteneur ZIP et ouverture par calamine, ou XML d'une feuille emballé par la cible dans une archive valide ; puis les deux usages du classeur, matrice de flux (jusqu'aux lignes à plat et au graphe) et fichier de labels (première feuille rendue en CSV). |

Le premier octet d'une entrée de `xlsx_reader` choisit sa forme : pair, le
reste est le classeur tel quel ; impair, le reste est le XML de la première
feuille, suivi au besoin du marqueur `<!--LUNAR-SHARED-STRINGS-->` (seul sur
sa ligne) et du XML des chaînes partagées. La cible fabrique alors l'archive :
une archive compressée mutée à l'aveugle est refusée dès son conteneur.

## Lancer

`cargo-fuzz`, la toolchain nightly du crate et un compilateur C++ sont requis :

```shell
script/ci/install-cargo-fuzz.sh        # cargo-fuzz et nightly, versions figées
script/fuzz/smoke.sh                   # toutes les cibles, 90 s chacune
FUZZ_SECONDS=600 script/fuzz/smoke.sh pcap_reader
```

Pour une campagne libre, depuis ce répertoire (rustup y applique la toolchain
nightly de `rust-toolchain.toml`) :

```shell
script/fuzz/prepare-corpus.sh
cd crates/lunar-flows-core/fuzz
cargo fuzz run --target x86_64-unknown-linux-gnu pcap_reader -- -max_len=262144
```

`--target` n'est pas facultatif avec le binaire officiel de cargo-fuzz : il
est compilé pour musl et vise cette cible par défaut, que les sanitizers
refusent. `script/fuzz/smoke.sh` donne de lui-même la cible de l'hôte.

La CI exécute les quatre cibles avec un budget borné à chaque push (job
« Fuzzing borné » de `ci.yml`) ; un crash y dépose son entrée dans l'artefact
`fuzz-artifacts`.

## Corpus et régressions

Le corpus (`corpus/`) et les entrées de crash (`artifacts/`) ne sont pas
versionnés. `script/fuzz/prepare-corpus.sh` reconstruit le corpus initial à
partir des fixtures du dépôt : petites captures du corpus TShark, matrices
LFMS de chaque type de liaison, relevés WinAudit dans chaque encodage accepté,
trames extraites des captures pcap (`script/fuzz/extract-frames.py`), et
quelques entrées malformées déterministes.

Seules les entrées qui reproduisent un défaut corrigé sont versionnées, dans
`regressions/<cible>/` ; le script les recopie dans le corpus, pour qu'un
défaut corrigé ne revienne pas.

- `pcap_reader/timestamp_overflow_*.pcap` : horodatages pcapng négatifs qui
  faisaient paniquer `timeval_to_systemtime`.
- `matrix_reader/packet_count_overflow.csv` : comptes de paquets dont la somme
  dépasse `u64::MAX` (entre tunnels d'une ligne, entre lignes d'un flux, entre
  flux d'un équipement). Les additions débordaient ; elles saturent, comme
  celles des octets. Défaut repéré à la relecture en écrivant la cible, que
  le fuzzer seul n'atteint pas vite : la graine le lui donne.

- `xlsx_reader/cellules_aux_deux_bouts.feuille` : feuille de 300 octets qui
  ne déclare que `A1` et `XFD1048576`. La plage rectangulaire de calamine
  réservait toute la surface entre les deux, soit 512 Gio : l'import d'un tel
  classeur arrêtait l'application. Trouvé par la cible en vingt secondes. La
  feuille est maintenant lue cellule par cellule, dans des bornes (256
  colonnes, 1 048 576 lignes, 8 millions de cellules).
- `xlsx_reader/reference_de_cellule_debordante.feuille` : référence de
  cellule `AAAAAAAAAA2`, dont calamine 0.38.0 calcule la colonne sans
  contrôle de débordement. **Défaut amont, non corrigé** : panic dans une
  compilation à contrôles de débordement (tests, fuzzing) ; dans le binaire
  livré, compilé sans, la colonne aberrante est refusée par les bornes
  ci-dessus. La cible tolère ce seul panic, reconnu à son emplacement et à
  son message ; il faudra retirer cette tolérance quand calamine sera
  corrigé.

## Dépendances

Ce crate est un workspace à part : `libfuzzer-sys` reste hors du `Cargo.lock`
du produit, de son SBOM et de `cargo deny`. Son propre `Cargo.lock` doit porter
les mêmes versions que celui de la racine pour les crates communs, sans quoi le
fuzzing porterait sur autre chose que le code livré. La CI le vérifie ; après
une mise à jour des dépendances de `lunar-flows-core` :

```shell
script/fuzz/sync-lock.sh
```
