# WOOMY — REALIGNEMENT, AUDIT DE LIAISON ET FONDATION SÉMANTIQUE

Jalon vérifié le **2 octobre 2026**. Woomy est le **Wii U Game Reconstruction & Recompilation Framework**. Le produit cible est du code PC natif reconstruit depuis le dump utilisateur, avec MANMENMI pour les services plateforme ; le PPC exécuté par Woomy reste un outil d'analyse/validation. Le jeu ne démarre pas. La [baseline M9](STATUS_M9_BASELINE.md) est conservée, pas remplacée.

## Baseline reproduite et contrôles finaux

| Contrôle | Baseline M9 reproduite | Après ce jalon |
| --- | ---: | ---: |
| Tests workspace | 125 réussis, 0 échec, 2 ignorés | **134 réussis, 0 échec, 2 ignorés** |
| Test externe M9, lancé séparément | 1 réussi | **1 réussi** |
| PPC/IR historique | 512 MATCH | **512 MATCH** |
| Natif scalaire | 5 120 MATCH | **5 120 MATCH** |
| Multi-fonctions guest | 6 144 MATCH | **6 144 MATCH** |
| Quatre fonctions réelles, états synthétiques | 4 × 512 MATCH | **4 × 512 MATCH** |
| Originaux | 615 intacts | **615 intacts** |

`fmt --check`, Clippy `-D warnings`, suite complète, build release et tous les validateurs passent. Journaux `target/realignment-baseline.{json,log}` et `target/realignment-final.{json,log}`. Les quatre fonctions restent `027967CC`, `02798514`, `02838B10`, `02852BB4` du module Gambit ; 2 048 comparaisons PPC/IR/natif réussies, zéro mismatch/inconclusive dans ce domaine synthétique.

Neuf nouveaux tests : deux audits d'indéfinis indépendants du nom/faible/zéro, limites et identités dupliquées ; six tests sémantiques de champs/preuves, conflits signés, pile/base remplacée, overlap, effets inconnus, versions/déterminisme/sérialisation ; une propriété sur **128 paires d'offsets synthétiques**. Le test d'intégration persistance existant vérifie aussi l'enregistrement, la redérivation après corruption d'un ancien produit et la catégorie UNKNOWN d'OSReport. Aucun octet Nintendo dans les fixtures. Aucun fuzzing nouveau revendiqué.

## Résultat de l'enquête `$UNDEF`

**UNKNOWN**, zéro résolution nouvelle. Les 17 références ciblent toutes le record 3 de `.symtab` 35 : GLOBAL/OBJECT, section 0, value/size/other à zéro, addend zéro. Toutes sont dans `.rela.text` 7, cible `.text` 2 : **7 ADDR16_HA, 7 ADDR16_LO, 3 REL24**. Quatorze sont dans le voisinage de l'entrée, trois à `0x02C86EFE..0x02C86F08`. Le détail de chaque record, voisins, symboles et FILEINFO est dans [LINKING.md](LINKING.md) et le rapport local d'audit.

Les sources publiques épinglées Decaf/Cemu ignorent la section zéro lors du fixup des symboles puis consomment la valeur conservée pendant la relocation. Cela soutient une hypothèse de comportement, sans établir une règle Cafe retail applicable. Le symbole est GLOBAL, pas WEAK ; il ne désigne aucune section d'import. FILEINFO fournit des métadonnées de layout, aucune liaison démontrée. Un seul RPX est présent dans le corpus sélectionné ; aucun RPL fournisseur ni export correspondant n'est établi. L'hypothèse inter-module reste non prouvée.

Évidence locale, observations publiques et inférences sont séparées. Il manque une règle légitime du producteur/chargeur, un link map authentique ou une observation reproductible du chargement du record exact. Aucune adresse inventée, valeur zéro appliquée, instruction sautée, modification du dump ou faux service. Le loader conserve sa politique ; l'amélioration générique est l'audit persistant des SHN_UNDEF, pas une nouvelle liaison.

## Changements livrés

- `crates/core/src/linking.rs` : audit versionné, exact et borné, toutes références indéfinies et leurs métadonnées ; aucune condition sur le nom Splatoon ou `$UNDEF`.
- `crates/core/src/semantic.rs` : modèle persistant et première passe au-dessus de Typed IR, registre/offsets locaux, champs/agrégats candidats, usages de types et catégories d'import, preuves et conflits.
- `crates/core/src/lib.rs`, `planning.rs`, `crates/cli/src/main.rs` : commandes **`image link-audit [module-id]`** et **`semantic [function-id]`**, aide correspondante.
- `crates/core/tests/semantic.rs`, `cafe_context.rs` : tests synthétiques, propriétés et intégration/persistance.
- Documentation : [DIRECTION.md](DIRECTION.md), [LINKING.md](LINKING.md), [SEMANTICS.md](SEMANTICS.md), [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md), README, CONTEXT/CALLS/NATIVE/PROCESS/SOURCES et ce rapport.

SQLite reste **6**, IR **3**, backend C++ **3**, Native ABI **2**, runtime ABI **2**, image/loader/relocation/context **1/1/1/1**, process/thread/provider/import semantics **1/1/1/1**. Nouveaux produits **LINK_AUDIT 1** et **SEMANTIC 1** dans `analysis_products`, hashes/module/IR/version dans leurs identités. Aucune migration ni modification des contrats existants. Les commandes redérivent les produits courants ; les hypothèses stockées ne sont pas réutilisées comme faits. Les **18 artefacts natifs** M9 restent courants : aucune reconstruction inutile avec des versions inchangées.

## Progression machine globale

| Mesure | État | Delta de ce jalon |
| --- | ---: | ---: |
| Fonctions connues | 115 | 0 |
| LIFTED / PARTIALLY_LIFTED / INVALID | 18 / 97 / 0 | 0 / 0 / 0 |
| Instructions / blocs / edges connus | 5 187 / 1 283 / 1 658 | 0 |
| Éligibles natives / compilées | 18 / 18 | 0 / 0 |
| Indirects hors retours résolus / non résolus | 0 / 41 | 0 / 0 |
| Relocations appliquées / différées / unsupported | 393 068 / 9 423 / 17 sur 402 508 | 0 |
| Contexte réel prêt / match de référence | 0 / 0 | 0 / 0 |
| Handlers de modèle / validés contre référence | 2 / 0 | 0 / 0 |

Couverture et fermeture sont **identiques aux rapports M9**. La commande analyze a revérifié le dump et réutilisé le module compatible : aucun reparsage forcé. CafeGuestImage est reconstruite depuis le RPX et vérifiée, hash inchangé `77fe959643e3db31964dd743fce074884cdb373b0216edd9b9063076cbb5d273`. Six régions, 16 069 688 octets. Fermeture non native : 115 fonctions connues, 97 partielles, 51 obstacles de contrôle ; 35 imports distincts / 70 sites, 459 appels au total. Les 167 lignes d'indirects, retours inclus, sont identiques à la baseline. Les quatre opcodes UNKNOWN ne sont pas modifiés.

Les 18 contextes de référence restent incomplets (4 register-only, 14 nécessitant mémoire). Il n'existe toujours aucune capture Cafe réelle légitime dans les artefacts disponibles. Les snapshots et la distinction synthétique/référence sont conservés ; aucune nouvelle référence ni équivalence réelle n'est annoncée.

## Résultats sémantiques mesurés

Domaine **STATIC_MACHINE_IR_HYPOTHESES**, corpus de **115 fonctions** et **25 675 opérations Typed IR** (pas 25 675 instructions PPC). Les candidats sont locaux aux blocs et peuvent représenter plusieurs vues d'un même objet ou des tableaux plutôt que des structures.

| Mesure | Numérateur / dénominateur ou périmètre |
| --- | --- |
| Fonctions analysées | **115 / 115** fonctions connues |
| Fonctions avec candidats de type | **104 / 115** |
| Candidats de type / conflits | **810 candidats, 1 conflit** dans les 115 rapports |
| Signatures candidates | **107 / 115** blocs d'entrée ; zéro signature/ABI prouvée |
| Agrégats candidats | **91** dans les 115 fonctions ; zéro identité source prouvée |
| Champs candidats | **244** dans ces 91 agrégats ; **1 825 accès mémoire IR** examinés |
| Imports classés | **21 / 70 sites** ; les 49 autres restent UNKNOWN |
| Opérations sémantiques candidates | **1 036 / 25 675 opérations IR** examinées |
| Vtables / classes / appels virtuels résolus | **0**, détecteurs non implémentés dans v1 |
| Ancres PPC/IR des rapports persistés | **115 / 115 rapports vérifiés** |

L'inférence conserve alternatives, statuts et preuves. Elle ne déduit pas de signedness à partir de la largeur, ni nom de champ, nom métier, héritage, ownership ou ABI générale. Les listes objet/vtable/classe et les opérations futures sont conçues dans le modèle mais ne sont pas des détecteurs livrés. Les signatures sont des observations de registres lus dans le bloc d'entrée, incluant état de contrôle/pile ; elles ne prouvent pas les paramètres Cafe. La confiance numérique reste null. Aucun candidat sémantique ne résout encore un indirect ou ne pilote codegen.

## Entry frontier et M9

| Profil | Instructions | Blocs terminés | Appels M9 réussis | Arrêt, identique à M9 |
| --- | ---: | ---: | ---: | --- |
| Debugger absent | **13** | **3** | **2** | avant `0x02C6BCF0`, `$UNDEF` au champ `0x02C6BCF2` |
| Debugger présent | **19**, trap inclus | **4** | **2** | trap `0x02C6BCEC` |

Les deux profils ont été rejoués : état PPC/IR concordant, cinq écritures de pile chacun, **INCONCLUSIVE**. Les services réellement appelés restent OSGetCurrentThread et OSIsDebuggerInitialized. Thread `0x70000000`, pile et debugger restent des valeurs de modèle, pas une capture de Splatoon. Le prochain obstacle sans debugger est une connaissance de liaison Cafe manquante, pas un service plateforme supplémentaire.

## Bridge natif et prochains travaux

Le [design du bridge natif](NATIVE_BRIDGE.md) précise ABI C, fixed-width values, handles opaques avec génération, adresses guest explicites, ownership, mémoire autoritaire, callbacks bornés, statuts/erreurs, version handshake, durées de vie, limites et absence d'exceptions traversant l'ABI. **DESIGN_ONLY** : aucun appel natif généré → M9 n'est livré par ce jalon. Les refus existants d'abandonner l'identité runtime restent actifs. Le runtime final visé n'interprétera pas du PPC.

Prochain jalon recommandé : propagation sémantique inter-blocs avec jointures/preuves conservatrices et premiers candidats function-pointer/table appuyés sur relocations et cibles exécutables ; en parallèle de conception, préparer un test synthétique du pont natif spécifié. La résolution machine ne consommera ces candidats qu'après preuve complète. Pour `$UNDEF`, rechercher la preuve de liaison manquante et conserver UNKNOWN tant qu'elle manque. Aucune réécriture manuelle de fonctions Splatoon ni implémentation prématurée GX2/audio/input/FS.

## Intégrité, régressions et artefacts

**615 fichiers originaux**, **1 761 602 316 octets**, **0 modifié/manquant/inattendu** avant/après. SQLite integrity_check=ok et aucune violation de clé étrangère. Les 22 sources/en-têtes M9 consommés gardent leurs empreintes. Aucun original ni fichier M9 modifié ; aucun commit créé ; dérivés sous `.woomy` ignoré.

Aucun mismatch détecté. Clippy a signalé l'usage de chunks_exact à taille constante dans l'audit ; corrigé avec as_chunks, sans désactiver le lint. Les tests, contraintes et états de preuve restent inchangés ou renforcés. Les limites sémantiques sont explicites ; aucune amélioration de couverture machine n'est fabriquée.

Les deux exécutables Woomy sont identiques : **10 648 064 octets**, SHA-256 **`b3f5a23f4a97491a6573bdd684e836c7e87bd1c10fcebbc6466311f06a5ca6cb`** (`target/release/woomy.exe` et `Splatoon/Games/Splatoon/woomy.exe`). Mode commande, refus de fonction absente, aide et shell CRLF vérifiés. Les 18 commandes finales ont réussi.

Rapports locaux : `.woomy/reports/realignment-final-summary.json`, `realignment-cli.json`, `realignment-audit.json`, `semantic-summary.json`, `realignment-semantic-entry.json`, `realignment-entry-{absent,present}.json`, `realignment-integrity-after.json`, `realignment-invariants.json`, `realignment-executables.json`. Les produits sémantiques détaillés sont persistés par fonction dans SQLite.

`Semantic inference != proven source-level identity` — `Confidence != evidence` — `PPC execution in Woomy = analysis/validation mechanism` — `PPC execution in final product = NOT THE TARGET ARCHITECTURE` — `Progress stops where evidence stops`.
