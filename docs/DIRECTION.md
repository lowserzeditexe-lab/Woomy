# Woomy — Wii U Game Reconstruction & Recompilation Framework

Direction officielle après M9, 2 octobre 2026. Le dépôt et les jalons existants sont conservés. Splatoon est le premier corpus réel ; les améliorations doivent être génériques et réutilisables par d'autres jeux Wii U.

Le produit visé est un exécutable PC natif construit localement depuis le dump fourni par l'utilisateur : analyse RPX/RPL → reconstruction → recompilation statique → code hôte natif → MANMENMI → services PC. L'exécution PPC dans Woomy est un mécanisme d'analyse/validation. **Le produit final ne doit pas dépendre d'un interpréteur PPC ni d'un JIT PPC.** Ce produit n'est pas encore réalisé.

## Trois axes convergents

| Axe | Travail | Preuve de progression |
| --- | --- | --- |
| A — machine et liaison | Cafe/ELF, relocations, découverte, CFG, dataflow, call graph, indirects, Typed IR, fermeture et éligibilité native | nouvelles règles générales, fixtures indépendantes, effets mesurés sur toutes les occurrences |
| B — reconstruction sémantique | types, signatures, pointeurs, tableaux, champs, agrégats, objets, vtables/classes, Semantic IR puis C/C++ reconstruit | hypothèses persistées, traces vers PPC/Typed IR, conflits conservés, absence de noms/identités inventés |
| C — exécution native et MANMENMI | code généré appelant une frontière stable vers CafeServiceRegistry et les services PC | ABI documentée, mémoire autoritaire, identités préservées, tests natifs complets avant toute annonce d'intégration |

La boucle de développement est : obstacle réel → classification de la cause → amélioration générique → tests synthétiques/unitaires/de propriété/régression → réanalyse → mesure globale. Le chemin d'entrée est une sonde d'intégration, pas l'architecture du projet. Les correctifs fonction par fonction et les scripts ad hoc de startup ne sont pas la stratégie.

Chaque obstacle doit être classé : limitation générique Woomy, connaissance de liaison Cafe manquante, analyse sémantique manquante, service MANMENMI manquant, contexte réel manquant, ou comportement réellement spécifique au jeu. Une adaptation spécifique reste séparée du moteur et conserve sa provenance sous `.woomy`.

## Frontières de responsabilité

Woomy possède analyse RPX/RPL, CafeGuestImage, PPC/CFG/dataflow/call graph/XREF, Typed IR, Semantic IR, codegen, état CPU, GuestMemory, exécution de validation et preuves. MANMENMI est la **Native PC Compatibility Layer for Recompiled Wii U Games** ; ses implémentations C++ possèdent runtime, processus, threads et services. Ces systèmes ne sont pas recréés en Rust.

Typed IR reste la représentation autoritaire du comportement bas niveau. Semantic IR interprète cette représentation et ne la remplace pas. À terme, les preuves sémantiques pourront aider la résolution des appels indirects ; aucun retour d'hypothèse non démontrée vers une cible machine certaine n'est permis.

Le provider de validation PPC/IR consomme maintenant l'ABI C M10. Le [pont natif](NATIVE_BRIDGE.md) est implémenté pour un sous-ensemble explicite de validation avec identité transportée. Les refus du backend ordinaire face à une identité externe non transportée restent actifs.

## Mesures et états

Les mesures publient domaine, numérateur et dénominateur : fonctions découvertes/analysées/levées/éligibles, blocs et instructions du corpus connu, indirects résolus/non résolus, signatures candidates/provées, fonctions avec types candidats, agrégats/champs/vtables/classes, appels virtuels résolus, imports catégorisés, couverture Semantic IR, fermeture native, handlers de modèle/de référence et contextes réels validés. Aucun pourcentage de bootabilité.

COMPILED, CONTEXT_ANALYZED, CONTEXT_INCOMPLETE, EXECUTION_READY, EXECUTED, DIFFERENTIAL_MATCH_SYNTHETIC_DOMAIN, RUNTIME_EXECUTABLE, REAL_GAME_CONTEXT_MATCH, MISMATCH et INCONCLUSIVE restent distincts. Un contexte modèle et un accord PPC/IR n'établissent pas la justesse Cafe réelle. Une absence de capture légitime n'empêche pas le travail générique de liaison, d'analyse ou de génération native.

Woomy Studio pourra parcourir les mêmes preuves, identités et produits persistés : fonction → PPC → CFG → Typed IR → Semantic IR → pseudo-C++ → C++ natif. Aucune UI ou vue de reconstruction fictive n'est ajoutée ici.

## Références d'architecture et distribution

[WiiCompiled](https://github.com/patchzyy/wiicompiled) illustre la séparation traducteur/runtime, la sortie native et la construction locale depuis des données utilisateur. [Aurora](https://github.com/encounter/aurora) est une couche de compatibilité source GameCube/Wii ; son organisation inspire la frontière plateforme, pas des règles Cafe. Ces architectures ne sont pas des spécifications Wii U et aucun code n'est incorporé. « Dusklight » reste une inspiration d'organisation mentionnée dans la direction utilisateur ; aucun dépôt exact n'étant identifié, aucune affirmation technique ni dépendance n'en est déduite.

Les originaux `code/content/meta` restent en lecture seule. Aucun téléchargement de jeu/clé, binaire propriétaire, C++ généré du jeu ou snapshot réel n'est distribué. Les dérivés sont locaux sous `.woomy` ignoré. La même séparation s'applique au produit natif futur.

## Invariants

Unknown != 0. Guest address != host pointer. CafeGuestImage != live Cafe process. Manmenmi model != reference capture. Handler implemented != Cafe semantics proven. RUNTIME_EXECUTABLE != REAL_GAME_CONTEXT_MATCH. Compiled != semantically validated. Semantic inference != proven source-level identity. Confidence != evidence. Progress stops where evidence stops.
