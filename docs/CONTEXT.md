# Contextes, domaines et dépendances

Une image fournit du stockage initial et des cibles liées. Un contexte d'exécution exige aussi des registres, une pile éventuelle, des objets du processus, les dépendances d'appel et des limites justifiées. La compilation ne fournit aucune de ces entrées.

## Analyse des prérequis

`ExecutionContextBuilder` parcourt l'IR vérifié. Un point fixe conservateur recherche les registres lus avant définition sur les chemins du CFG connu, y compris LR, CTR, CR, XER, FPSCR et FPR. Les effets inconnus et appels ne prouvent pas une définition. Les expressions mémoire restent limitées aux constantes, adresses relocalisées et offsets symboliques par bloc ; un alias non démontré reste inconnu. Aucune variable ou structure C n'est inventée.

Chaque exigence porte la fonction, le bloc, le PC PPC et l'opération IR lorsqu'elle existe : lecture de registre, accès de pile, lecture/écriture globale, cible de relocation, appel interne/importé/indirect ou effet inconnu. Un global mutable initialisé par le chargeur ne prouve pas sa valeur au moment d'un appel réel. Les offsets de r1 n'autorisent pas l'allocation d'une pile arbitraire. Les limites de chemins et les frontières de fonctions inférées restent explicites.

Les valeurs ont une provenance distincte : LoaderDerived, ZeroInitializedByLoader, RelocationDerived, ExplicitTestInput, ReferenceSnapshot, RuntimeProvided ou Unknown. L'analyse de contexte ne transforme jamais Unknown en zéro. `COMPILED`, `CONTEXT_INCOMPLETE`, `EXECUTION_READY`, `EXECUTED` et les résultats différentiels décrivent des étapes différentes, toujours assorties de leur domaine.

## Validation d'une fonction réelle dans un domaine synthétique

```text
context <function-id>
validate-function <function-id> synthetic
native status <function-id>
```

La seconde commande est une demande explicite d'exécution. Le sous-ensemble actuel exige une fonction fermée sans mémoire de données, appel ni cible inconnue, une image vérifiée, des instructions entièrement prises en charge et un registre de cibles complet. Les blocs exécutables sont extraits de l'image. Les registres sont des entrées de test explicites ; LR=0xFFFF0003 désigne le retour externe du harnais, pas un appelant Cafe observé.

Le corpus déterministe comprend 512 états, graine `0x62A7C39D`, valeurs entières limites, motifs et valeurs pseudo-aléatoires, FPR conservés en bits et registres spéciaux. Chaque état dispose de 512 pas, zéro appel autorisé, profondeur zéro, trace d'appels limitée à 16 et mémoire bornée. Le code natif provient d'une compilation fraîche de l'IR vérifié dans un répertoire privé, jamais d'un C++ éditable relu depuis le disque.

Les trois exécutions comparent l'état complet : GPR/FPR/CR/LR/CTR/XER/FPSCR, backing mémoire, écritures ordonnées, trace d'instructions, événements d'appel et terminaison. Un arrêt identique mais incomplet reste INCONCLUSIVE ; une différence reste MISMATCH. Le rapport conserve tous les états initiaux et les résultats reproductibles.

Le domaine est **GAME_FUNCTION_SYNTHETIC_DOMAIN** et son résultat positif **DIFFERENTIAL_MATCH_SYNTHETIC_DOMAIN**. Il ne rend jamais la fonction prête pour l'exécution réelle du jeu. Le domaine **GAME_FUNCTION_REFERENCE_CONTEXT** et **REAL_GAME_CONTEXT_MATCH** sont réservés à une capture légitime rejouée et comparée. Aucun résultat de ce second domaine n'est actuellement revendiqué.

## Fermeture des dépendances

`closure <function-id>` suit les cibles internes connues, y compris les cycles, avec une limite de 4096 fonctions. Il énumère fonctions éligibles/partielles, diagnostics IR, mémoire requise, contextes incomplets, imports et sorties non résolues. Les chemins de première accessibilité sont déterministes. Une sortie inconnue peut cacher d'autres dépendances : le rapport n'est pas une preuve de complétude du jeu.

Les états BLOCKED_PARTIAL_FUNCTION, BLOCKED_UNSUPPORTED_IR, BLOCKED_UNKNOWN_IMPORT, BLOCKED_INDIRECT et BLOCKED_CONTEXT sont cumulables. Aucun CLOSED_NATIVE ou CLOSED_WITH_RUNTIME_IMPORTS n'est annoncé tant que le contexte et les services ne sont pas justifiés.

`imports catalog` regroupe les sites par bibliothèque/symbole. Le rapport `reports/required-imports.json`, produit par `closure`, ajoute les chemins, effets connus/inconnus, besoins de provider et absence d'implémentation. Il classe les imports par longueur du chemin connu, nombre de sites puis identité stable. Une proximité statique n'est pas une preuve d'ordre dynamique. L'adaptateur M10 est implémenté ; deux contrats de modèle sont exposés séparément. Les effets Cafe de référence et les conditions de fermeture restent non prouvés.

## Snapshots Woomy version 1

Le format JSON indépendant des outils de capture contient : version, versions image/loader/relocation/contexte, module et SHA-256, identité d'image, fonction, entrée/CIA, GPR, FPR en bits, CR/LR/CTR/XER/FPSCR, origine/source, provenance des registres, régions avec base/permissions/type/octets/SHA-256/provenance, et empreinte facultative d'un état de sortie attendu.

Les origines sont SYNTHETIC, CEMU_REFERENCE, WIIU_REFERENCE et EXTERNAL_REFERENCE. L'origine externe est une déclaration, pas une authentification. Les snapshots SYNTHETIC ne deviennent pas des références du jeu. Une référence doit déclarer ReferenceSnapshot pour tous les registres et toutes les régions fournis. Aucune capture n'est fabriquée et aucun outil Cemu n'est requis.

```text
snapshot inspect snapshots/capture.json
snapshot validate snapshots/capture.json
```

Les chemins sont relatifs à `.woomy` et passent par SafeOutput ; traversée, lien/reparse dangereux et dépassement sont rejetés. Le fichier est limité à 16 Mio, les régions à 256 et leur backing total à 1 Mio. Le parseur refuse les champs inconnus, versions périmées, tailles/ranges/chevauchements/permissions invalides, hashes incohérents, CR invalides et identités erronées. CIA doit être l'entrée de fonction ; une reprise arbitraire au milieu d'un bloc n'est pas prise en charge. La validation liée à l'image compare chaque mot exécutable à ses octets dérivés.

L'import, les métadonnées, la construction d'un `Machine` et `snapshot replay` sont implémentés pour les fonctions dont l'état fourni suffit. Replay compare PPC/IR/natif si disponible et l'empreinte facultative de sortie. La capture externe, son attestation et le sidecar process M9 pour replay restent à fournir ; voir [PROCESS.md](PROCESS.md). `snapshot validate` contrôle structure et identité ; il n'annonce ni exécution ni équivalence. Les contrôles d'identité de programme et de contexte gardent ces étapes séparées.

Les données de jeu, C++/binaires dérivés et éventuels snapshots restent dans `.woomy` ignoré. Les fixtures du dépôt sont exclusivement synthétiques.

Le domaine CAFE_BOOTSTRAP_MODEL est distinct des 18 contextes de référence toujours incomplets. Le modèle thread/pile/debugger ne complète pas rétroactivement leurs entrées inconnues. Les 4 contextes register-only restent éligibles au domaine synthétique, les 14 autres nécessitent une mémoire justifiée.

## Après M9 : domaines de reconstruction

La [direction](DIRECTION.md) traite la référence Cafe comme une source de preuve parmi plusieurs. Les captures ne bloquent pas les améliorations génériques. `semantic` produit des hypothèses statiques avec identité d’IR et provenance, distinctes de la préparation d’un contexte exécutable. `image link-audit` conserve les liaisons indéfinies et leurs preuves manquantes. Aucun candidat de champ ou signature ne remplit une valeur runtime inconnue.

## Convergence native M10

`entry run-model` compare maintenant PPC, IR et un préfixe C++ généré/compilé x86-64 utilisant l'ABI C M10. Le protocole dédié conserve les identités programme/processus/mémoire/état initial. Les barrières de relocation sont respectées avant fetch. Cet artefact de validation ne rend pas la fonction partielle éligible à codegen normal et ne fournit aucune capture réelle. Voir [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md) et [STATUS.md](STATUS.md). Semantic v2 ajoute des faits inter-blocs et des candidats de stockage sans modifier les valeurs runtime ni les cibles machine.


## Reconstruction interprocédurale v3

Le domaine sémantique ajoute des contraintes d'arguments/retours et un suivi des définitions de stockage à chaque lecture. Les entrées d'une fonction restent symboliques ; une image initialisée ne fournit pas un état mémoire mutable à une entrée runtime arbitraire. Les observations d'agrégat, callback, vtable et classe restent des hypothèses. Une cible unique ne peut rejoindre l'IR machine et le graphe d'appels qu'après redérivation et contrôle de preuve complet. La trace conserve le type de transfert indirect d'origine. Les produits dépendants sont invalidés et les résumés recalculés après promotion. Cette opération ne fournit ni contexte Cafe réel ni validation de référence. Voir [SEMANTICS.md](SEMANTICS.md).

## Résultat vérifié du jalon v3 — 4 octobre 2026

161 tests workspace réussis, deux tests externes M10 et toutes les campagnes différentielles historiques réussis. Analyse des 115 fonctions convergée en six tours, rapports reproductibles : 26 971 nœuds de provenance, 618 contraintes interprocédurales, six agrégats et 23 observations de champs. Les 41 indirects restent non résolus (27 effets d’appel, un alias, neuf valeurs inconnues et quatre motifs non pris en charge). Zéro nouvelle cible machine, zéro objet/vtable/classe/callback sur ce corpus ; les détecteurs disposent de tests synthétiques positifs. Les 17 $UNDEF, les frontières d’entrée et les 18 fonctions natives éligibles restent inchangés. Le dump et M10 sont intacts ; SQLite est intègre. Le [rapport A–AI](STATUS.md) conserve les dénominateurs, limites et hashes. Prochaine direction : effets mémoire interprocéduraux paramétriques et frontières IR manquantes, sans extension spéculative de M10.

## Travail Semantic v4

Le jalon v4 compose des effets mémoire relatifs aux origines entrantes et affine les clobbers par des comparaisons de plages conservatrices. Les bases décalées par constante sont propagées. La nouvelle forme Typed IR 4 de bclr conditionnel sans lien conserve LR/CTR et le chemin de continuation ; sa cible reste inconnue en l'absence de preuve. Les rapports v3 restent archivés dans [STATUS_V3_BASELINE.md](STATUS_V3_BASELINE.md). Voir [SEMANTICS.md](SEMANTICS.md) pour la distinction entre couverture complète des effets, adresses précises et contexte runtime réel.
